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
