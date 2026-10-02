//! Common configured-helper timeout wording; no HTTP or SSH classification.
use gwz_transport::protocol::{Effect, ErrorCode, Failure, SetupFailureCause};

pub(crate) const HELPER_INTERACTION_PREFIX: &str = "No credential helper answered within ";
pub(crate) const HELPER_ALLOCATION_PREFIX: &str = "No credential helper could start in the ";
pub(super) const MEMBER_URL: &str = "(`git -C <path> remote get-url origin` prints it; before the member is cloned, use the manifest's URL, or under `--url-scheme https` the one `gwz --verbose materialize --lock` prints after `->`)";

fn exact_seconds(ms: i64) -> String {
    let mut seconds = format!("{}.{:03}", ms / 1_000, ms % 1_000);
    while seconds.ends_with('0') {
        seconds.pop();
    }
    if seconds.ends_with('.') {
        seconds.pop();
    }
    seconds
}

pub(super) fn timeout_budget(failure: &Failure) -> Option<(SetupFailureCause, i64)> {
    let detail = failure.detail.as_ref()?;
    let ms = detail.helper_budget_ms?;
    let bounded = match failure.setup_cause {
        Some(SetupFailureCause::Interaction) => (1..=120_000).contains(&ms),
        Some(SetupFailureCause::Allocation) => (0..=86_400_000).contains(&ms),
        _ => false,
    };
    if failure.code != ErrorCode::Timeout
        || failure.effect != Effect::None
        || !bounded
        || detail.helper_cause.is_some()
        || detail.pipe_kind.is_some()
        || detail.schemes.is_some()
    {
        return None;
    }
    Some((failure.setup_cause?, ms))
}

pub(super) fn timeout_reason(failure: &Failure) -> Option<String> {
    let (cause, ms) = timeout_budget(failure)?;
    let seconds = exact_seconds(ms);
    Some(match cause {
        SetupFailureCause::Interaction => format!(
            "{HELPER_INTERACTION_PREFIX}{seconds} seconds, a fixed bound, so gwz gave up on it. If a helper is waiting for you to sign in or unlock it, sign in once with `git ls-remote` and this member's URL {MEMBER_URL}, then retry."
        ),
        _ => format!(
            "{HELPER_ALLOCATION_PREFIX}{seconds} seconds available to wait for resources. Other helpers may be busy, waiting for sign-ins or stuck. Finish any open sign-in, or sign in once with `git ls-remote` and this member's URL {MEMBER_URL}, then retry. See Troubleshooting: HTTPS Credential Failure."
        ),
    })
}
