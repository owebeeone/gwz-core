//! Capture publication objects and URLs without changing local refs or config.
use super::*;

pub(super) fn prepare(
    backend: &Git2Backend,
    path: &Path,
    remote: &str,
    refspec: &str,
) -> ModelResult<GitPreparedPush> {
    let repo = repository_support::open_repo(path)?;
    let handle = find_remote(&repo, remote)?;
    let url = handle
        .pushurl()
        .map_err(git_error)?
        .unwrap_or(handle.url().map_err(git_error)?)
        .to_owned();
    transport_support::identity::for_remote(backend, Some(&repo), Some(remote), &url)?;
    let prefix = if refspec.starts_with('+') { "+" } else { "" };
    let plain = refspec.strip_prefix('+').unwrap_or(refspec);
    let (source, destination) = plain.split_once(':').unwrap_or((plain, plain));
    let invalid = || {
        ModelError::new(
            ErrorCode::InvalidRequest,
            "push refspec cannot resolve to concrete source objects and destination refs",
        )
    };
    if destination.contains(':') {
        return Err(invalid());
    }
    let mut refspecs = Vec::new();
    if plain == ":" {
        // Git's matching-branches form: only branches already at the destination.
        let advertised = backend.ls_remote_url(path, &url, remote, Some(path))?;
        for reference in repo.references_glob("refs/heads/*").map_err(git_error)? {
            let reference = reference.map_err(git_error)?.resolve().map_err(git_error)?;
            let name = reference.name().map_err(git_error)?;
            if advertised.iter().any(|row| row.name == name) {
                refspecs.push(format!(
                    "{prefix}{}:{name}",
                    reference.target().ok_or_else(invalid)?
                ));
            }
        }
    } else if source.contains('*') || destination.contains('*') {
        if source.matches('*').count() != 1
            || destination.matches('*').count() != 1
            || !source.starts_with("refs/")
            || !destination.starts_with("refs/")
        {
            return Err(invalid());
        }
        let (before, after) = source.split_once('*').expect("one wildcard");
        for reference in repo.references().map_err(git_error)? {
            let reference = reference.map_err(git_error)?.resolve().map_err(git_error)?;
            let name = reference.name().map_err(git_error)?;
            if let Some(middle) = name
                .strip_prefix(before)
                .and_then(|rest| rest.strip_suffix(after))
            {
                let target = destination.replacen('*', middle, 1);
                if !git2::Reference::is_valid_name(&target) {
                    return Err(invalid());
                }
                refspecs.push(format!(
                    "{prefix}{}:{target}",
                    reference.target().ok_or_else(invalid)?
                ));
            }
        }
    } else {
        let object = if source.is_empty() {
            None
        } else {
            Some(repo.revparse_ext(source).map_err(git_error)?)
        };
        let destination = if destination.starts_with("refs/") {
            destination.to_owned()
        } else {
            // Resolve remote shorthand first, then infer from the source namespace.
            let advertised = backend.ls_remote_url(path, &url, remote, Some(path))?;
            let matches: Vec<_> = advertised
                .iter()
                .filter(|row| {
                    row.name == format!("refs/heads/{destination}")
                        || row.name == format!("refs/tags/{destination}")
                })
                .collect();
            match matches.as_slice() {
                [reference] => reference.name.clone(),
                [] => {
                    let name = object
                        .as_ref()
                        .and_then(|(_, reference)| reference.as_ref())
                        .and_then(|reference| reference.name().ok())
                        .ok_or_else(invalid)?;
                    let namespace = if name.starts_with("refs/heads/") {
                        "refs/heads"
                    } else if name.starts_with("refs/tags/") {
                        "refs/tags"
                    } else {
                        return Err(invalid());
                    };
                    format!("{namespace}/{destination}")
                }
                _ => return Err(invalid()),
            }
        };
        if !git2::Reference::is_valid_name(&destination) {
            return Err(invalid());
        }
        let oid = object
            .map(|(object, _)| object.id().to_string())
            .unwrap_or_default();
        refspecs.push(format!("{prefix}{oid}:{destination}"));
    }
    refspecs.sort();
    Ok(GitPreparedPush {
        remote: remote.to_owned(),
        url,
        refspecs,
    })
}

/// The object `destination` last had on `remote`'s repository, as this
/// repository's remote-tracking ref records it (gwz-dev
/// `dev-docs/GwzUrlSchemePushPlan.md` §3.5). Local only. `None` unless every
/// condition holds: `destination` is a branch; the remote's push URL is absent
/// or reaches the same repository as its fetch URL; the remote maps
/// `refs/heads/*` into `refs/remotes/<remote>/*` with a forced fetch refspec,
/// as `git clone` and `git remote add` write it; and no other fetch refspec,
/// of this remote or another, can write into that namespace. The branch's
/// configured upstream plays no part.
pub(super) fn last_known_ref(
    _backend: &Git2Backend,
    path: &Path,
    remote: &str,
    destination: &str,
) -> ModelResult<Option<String>> {
    let Some(branch) = destination.strip_prefix("refs/heads/") else {
        return Ok(None);
    };
    let repo = repository_support::open_repo(path)?;
    let handle = match repo.find_remote(remote) {
        Ok(handle) => handle,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(error) => return Err(git_error(error)),
    };
    let fetch_url = handle.url().map_err(git_error)?;
    if let Some(push_url) = handle.pushurl().map_err(git_error)?
        && !(crate::git::same_repository(fetch_url, push_url)
            || crate::git::same_repository(push_url, fetch_url))
    {
        return Ok(None);
    }
    let namespace = format!("refs/remotes/{remote}/");
    let mapping = format!("+refs/heads/*:{namespace}*");
    let config = repo
        .config()
        .map_err(git_error)?
        .snapshot()
        .map_err(git_error)?;
    let mut entries = config
        .entries(Some(r"^remote\..*\.fetch$"))
        .map_err(git_error)?;
    let mut mapped = false;
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(git_error)?;
        // A fetch refspec that cannot be read might write anywhere.
        if !entry.has_value() {
            return Ok(None);
        }
        let (Ok(name), Ok(refspec)) = (entry.name(), entry.value()) else {
            return Ok(None);
        };
        let owner = name
            .strip_prefix("remote.")
            .and_then(|name| name.strip_suffix(".fetch"));
        if owner == Some(remote) && refspec == mapping {
            mapped = true;
        } else if writes_under(refspec, &namespace) {
            return Ok(None);
        }
    }
    if !mapped {
        return Ok(None);
    }
    match repo.find_reference(&format!("{namespace}{branch}")) {
        Ok(reference) => Ok(reference.target().map(|target| target.to_string())),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(error) => Err(git_error(error)),
    }
}

/// Whether a configured fetch refspec can write a ref under `namespace`, which
/// ends with `/`. A negative refspec, or one without a destination, stores
/// nothing. Git expands a destination outside `refs/`, so that one may write
/// anywhere, and a pattern's destination may write wherever its prefix leads.
fn writes_under(refspec: &str, namespace: &str) -> bool {
    let plain = refspec.strip_prefix('+').unwrap_or(refspec);
    let Some((_, destination)) = plain.split_once(':') else {
        return false;
    };
    if plain.starts_with('^') || destination.is_empty() {
        return false;
    }
    if !destination.starts_with("refs/") {
        return true;
    }
    match destination.split_once('*') {
        Some((prefix, _)) => prefix.starts_with(namespace) || namespace.starts_with(prefix),
        None => destination.starts_with(namespace),
    }
}

#[cfg(test)]
mod tests {
    use super::writes_under;

    /// Which configured fetch refspecs can write under `origin`'s namespace, the
    /// condition that keeps another refspec's refs from passing for `origin`'s.
    #[test]
    fn a_fetch_refspec_writes_under_a_namespace_its_destination_can_reach() {
        for (refspec, writes) in [
            ("+refs/heads/*:refs/remotes/origin/*", true),
            ("refs/heads/*:refs/remotes/origin/*", true),
            ("+refs/heads/main:refs/remotes/origin/main", true),
            ("+refs/*:refs/*", true),
            ("+refs/heads/*:refs/remotes/*", true),
            ("+refs/heads/*:refs/remotes/origin*", true),
            ("+refs/heads/*:remotes/origin/*", true),
            ("+refs/heads/*:refs/remotes/originals/*", false),
            ("+refs/heads/*:refs/remotes/upstream/*", false),
            ("+refs/heads/*:refs/heads/*", false),
            ("refs/heads/main", false),
            ("refs/heads/main:", false),
            ("^refs/heads/wip", false),
        ] {
            assert_eq!(
                writes_under(refspec, "refs/remotes/origin/"),
                writes,
                "{refspec}"
            );
        }
    }
}
