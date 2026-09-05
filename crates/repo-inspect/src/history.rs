//! `inventory_history`: every root whose object graph must survive.
//!
//! Design §5.1: "Protect all refs, HEAD (including detached), retained reflog
//! roots, annotated objects, native stashes and older stash entries."
//! Architecture §5 adds the coordination records of GWZ stash bundles, which
//! core decodes and hands over
//! ([`crate::LocalRepoInspector::with_coordination_roots`]).
//!
//! Two spellings this module fixes, both to keep the root list free of
//! duplicates that say nothing extra:
//!
//! - a `refs/tags/…` reference whose object is an annotated tag object is
//!   reported **once**, as `RootSource::AnnotatedTag`, at the tag object; the
//!   tag's target is an edge of that object, so a graph walk still reaches it.
//!   A lightweight tag stays a `RootSource::Ref`.
//! - a reflog entry is reported only when no earlier root already names its
//!   object, so `Reflog` roots are exactly the objects the reflog alone
//!   retains. The reported `index` is therefore the reflog's own index and
//!   may be sparse.

use std::collections::BTreeSet;

use git2::{ErrorCode, Repository};
use gwz_repo_contract::{
    ObjectFormat, ObjectId, Observation, ProtectedRoot, ProtectedRoots, RepositoryInfo, RootSource,
    UnknownKind, UnknownReason,
};

use crate::oid::to_contract_oid;
use crate::{CoordinationRoot, normalise_roots};

/// Namespaces holding refs that exist only while an operation is open. They
/// are protected in the repository being inspected, but design §5.1's
/// "existing temporary operation refs are not witnesses" excludes them from
/// [`retained_roots`], the witness side.
const TEMPORARY_NAMESPACES: &[&str] = &["refs/gwz/merge/"];

pub(crate) fn inventory_history(
    repository: &RepositoryInfo,
    coordination: &[CoordinationRoot],
) -> Observation<ProtectedRoots> {
    let opened = match crate::work::open(repository) {
        Ok(opened) => opened,
        Err(reason) => return Observation::Unknown(vec![reason]),
    };
    let mut collected = Collected::default();
    collected.gather(&opened, repository.object_format, false);
    for root in coordination {
        collected.roots.push(ProtectedRoot {
            source: RootSource::CoordinationRecord {
                record: root.record.clone(),
                object: root.object.clone(),
            },
            oid: root.oid.clone(),
        });
    }
    if collected.fatal {
        return Observation::Unknown(collected.unknown);
    }
    // Per-root unreadability is reported as a whole-observation unknown here:
    // unlike `WorkObservation`, `ProtectedRoots` has no per-entry unknown
    // channel, and a root list that silently omits a ref is not a safe
    // inventory (design §5.1: unknown evidence refuses).
    if !collected.unknown.is_empty() {
        return Observation::Unknown(collected.unknown);
    }
    Observation::Known(normalise_roots(collected.roots))
}

/// The reader's own roots, eligible as history witnesses.
pub(crate) fn retained_roots(
    repository: &Repository,
    format: ObjectFormat,
) -> Result<ProtectedRoots, String> {
    let mut collected = Collected::default();
    collected.gather(repository, format, true);
    if let Some(reason) = collected.unknown.first() {
        return Err(reason.detail.clone());
    }
    Ok(normalise_roots(collected.roots))
}

#[derive(Default)]
struct Collected {
    roots: Vec<ProtectedRoot>,
    seen: BTreeSet<ObjectId>,
    unknown: Vec<UnknownReason>,
    fatal: bool,
}

impl Collected {
    fn gather(&mut self, repository: &Repository, format: ObjectFormat, witnesses_only: bool) {
        let mut references = Vec::new();
        match repository.references() {
            Ok(iterator) => {
                for reference in iterator {
                    match reference {
                        Ok(reference) => {
                            let Ok(name) = reference.name() else {
                                self.unreadable(
                                    "a reference name is not valid UTF-8",
                                    String::from_utf8_lossy(reference.name_bytes()).into_owned(),
                                );
                                continue;
                            };
                            references.push(name.to_owned());
                        }
                        Err(error) => self.unreadable("a reference could not be read", error),
                    }
                }
            }
            Err(error) => {
                self.fatal = true;
                self.unreadable("the reference store could not be read", error);
                return;
            }
        }

        for name in &references {
            if witnesses_only && is_temporary(name) {
                continue;
            }
            let Some(oid) = self.resolve(repository, format, name) else {
                continue;
            };
            let annotated = name.starts_with("refs/tags/")
                && crate::oid::to_git_oid(&oid)
                    .ok()
                    .is_some_and(|id| repository.find_tag(id).is_ok());
            let source = if annotated {
                RootSource::AnnotatedTag {
                    name: name.to_owned(),
                }
            } else {
                RootSource::Ref {
                    name: name.to_owned(),
                }
            };
            self.push(source, oid);
        }

        match repository.head() {
            Ok(head) => {
                if let Some(oid) = self.target_of(&head, format) {
                    self.push(RootSource::Head, oid);
                }
            }
            Err(error) if matches!(error.code(), ErrorCode::UnbornBranch | ErrorCode::NotFound) => {
            }
            Err(error) => self.unreadable("HEAD could not be read", error),
        }

        self.gather_stashes(repository, format);

        let mut reflog_sources = vec!["HEAD".to_owned()];
        reflog_sources.extend(references.iter().cloned());
        for name in reflog_sources {
            if witnesses_only && is_temporary(&name) {
                continue;
            }
            self.gather_reflog(repository, format, &name);
        }
    }

    /// Native stash entries, newest first: `refs/stash`'s reflog is exactly
    /// the stash stack, so entry `n` is `stash@{n}` and the older entries
    /// design §5.1 names are the entries past 0.
    fn gather_stashes(&mut self, repository: &Repository, format: ObjectFormat) {
        let reflog = match repository.reflog("refs/stash") {
            Ok(reflog) => reflog,
            Err(error) if error.code() == ErrorCode::NotFound => return,
            Err(error) => {
                self.unreadable("the stash reflog could not be read", error);
                return;
            }
        };
        for index in 0..reflog.len() {
            let Some(entry) = reflog.get(index) else {
                continue;
            };
            let id = entry.id_new();
            if id.is_zero() {
                continue;
            }
            match to_contract_oid(format, id) {
                Ok(oid) => self.push(
                    RootSource::Stash {
                        index: index as u64,
                    },
                    oid,
                ),
                Err(detail) => self.unreadable("a stash entry id could not be read", detail),
            }
        }
    }

    fn gather_reflog(&mut self, repository: &Repository, format: ObjectFormat, name: &str) {
        let reflog = match repository.reflog(name) {
            Ok(reflog) => reflog,
            Err(error) if matches!(error.code(), ErrorCode::NotFound) => return,
            Err(error) => {
                self.unreadable(&format!("the reflog of {name} could not be read"), error);
                return;
            }
        };
        for index in 0..reflog.len() {
            let Some(entry) = reflog.get(index) else {
                continue;
            };
            let id = entry.id_new();
            if id.is_zero() {
                continue;
            }
            match to_contract_oid(format, id) {
                Ok(oid) if !self.seen.contains(&oid) => self.push(
                    RootSource::Reflog {
                        reference: name.to_owned(),
                        index: index as u64,
                    },
                    oid,
                ),
                Ok(_) => {}
                Err(detail) => self.unreadable("a reflog entry id could not be read", detail),
            }
        }
    }

    fn resolve(
        &mut self,
        repository: &Repository,
        format: ObjectFormat,
        name: &str,
    ) -> Option<ObjectId> {
        match repository.find_reference(name) {
            Ok(reference) => self.target_of(&reference, format),
            Err(error) => {
                self.unreadable(&format!("{name} could not be resolved"), error);
                None
            }
        }
    }

    fn target_of(
        &mut self,
        reference: &git2::Reference<'_>,
        format: ObjectFormat,
    ) -> Option<ObjectId> {
        let resolved = match reference.resolve() {
            Ok(resolved) => resolved,
            Err(error) if error.code() == ErrorCode::NotFound => return None,
            Err(error) => {
                self.unreadable("a symbolic reference could not be resolved", error);
                return None;
            }
        };
        let id = resolved.target()?;
        match to_contract_oid(format, id) {
            Ok(oid) => Some(oid),
            Err(detail) => {
                self.unreadable("a reference target could not be read", detail);
                None
            }
        }
    }

    fn push(&mut self, source: RootSource, oid: ObjectId) {
        self.seen.insert(oid.clone());
        self.roots.push(ProtectedRoot { source, oid });
    }

    fn unreadable(&mut self, what: &str, detail: impl std::fmt::Display) {
        self.unknown.push(UnknownReason::new(
            UnknownKind::Unreadable,
            format!("{what}: {detail}"),
        ));
    }
}

fn is_temporary(name: &str) -> bool {
    TEMPORARY_NAMESPACES
        .iter()
        .any(|namespace| name.starts_with(namespace))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_open_operation_namespace_is_temporary() {
        assert!(is_temporary("refs/gwz/merge/m1/root/head"));
        assert!(!is_temporary("refs/gwz/local-imports/t1"));
        assert!(!is_temporary("refs/heads/main"));
        assert!(!is_temporary("refs/stash"));
    }
}
