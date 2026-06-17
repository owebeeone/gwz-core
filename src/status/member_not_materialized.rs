
use crate::artifact::{LockArtifact, ManifestMember};
use crate::git::{GitHeadState, GitStatus as BackendGitStatus};
use crate::model::ModelError;



pub(crate) fn lock_match(
    lock: Option<&LockArtifact>,
    member: &ManifestMember,
    head: &GitHeadState,
    status: &BackendGitStatus,
) -> crate::LockMatch {
    let Some(lock) = lock else {
        return crate::LockMatch::Missing;
    };
    let Some(locked) = lock.members.get(&member.id) else {
        return crate::LockMatch::Missing;
    };
    if locked.commit == head.commit && locked.dirty.unwrap_or(false) == status.is_dirty {
        crate::LockMatch::Matches
    } else {
        crate::LockMatch::Differs
    }
}

pub(crate) fn member_not_materialized(
    member: &ManifestMember,
    source_kind: crate::SourceKind,
    lock: Option<&LockArtifact>,
) -> crate::MemberResponse {
    let locked = lock.and_then(|lock| lock.members.get(&member.id));
    crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: member.path.clone(),
        source_kind,
        status: crate::MemberStatus::Noop,
        error: None,
        planned: None,
        state: Some(crate::ResolvedMemberState {
            member_id: member.id.clone(),
            path: locked
                .map(|state| state.path.clone())
                .unwrap_or_else(|| member.path.clone()),
            source_id: member.source_id.clone(),
            source_kind,
            commit: locked.and_then(|state| state.commit.clone()),
            branch: locked.and_then(|state| state.branch.clone()),
            detached: locked.and_then(|state| state.detached),
            upstream: locked.and_then(|state| state.upstream.clone()),
            dirty: None,
            materialized: false,
            remotes: member
                .remotes
                .iter()
                .map(|remote| crate::RemoteSpec {
                    name: remote.name.clone(),
                    url: remote.url.clone(),
                    fetch: Some(remote.fetch),
                    push: Some(remote.push),
                })
                .collect(),
        }),
        git_status: None,
        lock_match: Some(crate::LockMatch::Missing),
    }
}

pub(crate) fn member_error(
    member: &ManifestMember,
    source_kind: crate::SourceKind,
    error: ModelError,
    status: crate::MemberStatus,
) -> crate::MemberResponse {
    crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: member.path.clone(),
        source_kind,
        status,
        error: Some(crate::GwzError {
            code: error.code.into(),
            message: error.message,
            member_id: Some(member.id.clone()),
            member_path: Some(member.path.clone()),
            detail: None,
        }),
        planned: None,
        state: None,
        git_status: None,
        lock_match: None,
    }
}

