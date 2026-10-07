use gwz_transport::protocol::{ErrorCode, GitService};
cfg_if::cfg_if! { if #[cfg(test)] { #[path="https_policy_tests.rs"] mod tests; } }
use std::{collections::BTreeMap, sync::Arc};
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ResponseAction {
    Success,
    Interim,
    Redirect,
    Fail(ErrorCode),
}
pub(crate) fn advertisement(service: GitService) -> bool {
    matches!(
        service,
        GitService::UploadPackAdvertisement | GitService::ReceivePackAdvertisement
    )
}
pub(crate) fn receive_pack(service: GitService) -> bool {
    matches!(
        service,
        GitService::ReceivePackAdvertisement | GitService::ReceivePackExchange
    )
}
pub(crate) fn service_name(service: GitService) -> &'static str {
    if receive_pack(service) {
        "git-receive-pack"
    } else {
        "git-upload-pack"
    }
}
pub(crate) fn response_type(service: GitService) -> String {
    format!(
        "application/x-{}-{}",
        service_name(service),
        if advertisement(service) {
            "advertisement"
        } else {
            "result"
        }
    )
}
pub(crate) fn classify(status: u16, service: GitService) -> ResponseAction {
    use ResponseAction::*;
    match status {
        100 | 102 | 103 => Interim,
        200 => Success,
        301 | 302 | 303 | 307 | 308 if advertisement(service) => Redirect,
        300..=399 => Fail(ErrorCode::UnsupportedOperation),
        401 | 407 => Fail(ErrorCode::Authentication),
        403 | 404 if advertisement(service) => Fail(ErrorCode::RepositoryRefused),
        400..=599 => Fail(ErrorCode::Io),
        _ => Fail(ErrorCode::Protocol),
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RouteKey {
    operation: String,
    original: String,
    receive: bool,
}
impl RouteKey {
    pub(crate) fn new(operation: &str, original: &str, service: GitService) -> Self {
        Self {
            operation: operation.into(),
            original: original.into(),
            receive: receive_pack(service),
        }
    }
}
/// The write-once routes of an endpoint's operations, one per (operation,
/// original URL, receive). A route is held until its whole operation finishes
/// and is never counted against a limit: a remote that discovered it has gone
/// before the exchange that needs it (adaptive concurrency design §7.2, F7).
pub(crate) struct Routes {
    routes: BTreeMap<RouteKey, Route>,
    missing: BTreeMap<String, gwz_transport::protocol::Failure>,
}
impl Routes {
    pub(crate) fn new() -> Self {
        Self {
            routes: BTreeMap::new(),
            missing: BTreeMap::new(),
        }
    }
    pub(crate) fn admit(&mut self, key: RouteKey) {
        self.routes.entry(key).or_insert_with(|| Route {
            pinned: None,
            challenge: None,
            answers: Arc::new(Default::default()),
            native: None,
            basic: false,
        });
    }
    pub(crate) fn install(&mut self, key: &RouteKey, base: &str) -> Result<(), ErrorCode> {
        let slot = &mut self
            .routes
            .get_mut(key)
            .ok_or(ErrorCode::InvalidRequest)?
            .pinned;
        if let Some(pinned) = slot {
            if pinned != base {
                return Err(ErrorCode::Protocol);
            }
        } else {
            *slot = Some(base.into());
        }
        Ok(())
    }
    pub(crate) fn get(&self, key: &RouteKey) -> Result<&str, ErrorCode> {
        self.routes
            .get(key)
            .and_then(|route| route.pinned.as_deref())
            .ok_or(ErrorCode::InvalidRequest)
    }
    cfg_if::cfg_if! { if #[cfg(test)] {
        pub(crate) fn len(&self) -> usize {
            self.routes.len()
        }
    } }
    pub(crate) fn finish(&mut self, operation: &str) {
        self.routes.retain(|key, _| key.operation != operation);
        self.missing.remove(operation);
    }
}

struct Route {
    pinned: Option<String>,
    challenge: Option<String>,
    answers: Arc<super::https_worker::credentials::Answers>,
    native: Option<Arc<super::https_worker::native::Authenticated>>,
    basic: bool,
}
impl Routes {
    pub(crate) fn basic_install(&mut self, key: &RouteKey) -> Result<(), ErrorCode> {
        self.routes
            .get_mut(key)
            .ok_or(ErrorCode::InvalidRequest)?
            .basic = true;
        Ok(())
    }
    pub(crate) fn basic_get(&self, key: &RouteKey) -> bool {
        self.routes.get(key).is_some_and(|route| route.basic)
    }
    pub(crate) fn native_install(
        &mut self,
        key: &RouteKey,
        authenticated: Arc<super::https_worker::native::Authenticated>,
    ) -> Result<(), ErrorCode> {
        self.routes
            .get_mut(key)
            .ok_or(ErrorCode::InvalidRequest)?
            .native = Some(authenticated);
        Ok(())
    }
    pub(crate) fn native_get(
        &self,
        key: &RouteKey,
    ) -> Option<Arc<super::https_worker::native::Authenticated>> {
        self.routes.get(key).and_then(|route| route.native.clone())
    }
    pub(crate) fn answers(
        &self,
        key: &RouteKey,
    ) -> Result<Arc<super::https_worker::credentials::Answers>, ErrorCode> {
        self.routes
            .get(key)
            .map(|r| r.answers.clone())
            .ok_or(ErrorCode::InvalidRequest)
    }
    pub(crate) fn challenge(&mut self, key: &RouteKey, base: &str) -> Result<(), ErrorCode> {
        self.routes
            .get_mut(key)
            .ok_or(ErrorCode::InvalidRequest)?
            .challenge = Some(base.into());
        Ok(())
    }
    pub(crate) fn challenged(&self, key: &RouteKey) -> Option<&str> {
        self.routes.get(key).and_then(|r| r.challenge.as_deref())
    }
    pub(crate) fn missing_git(&self, key: &RouteKey) -> Option<gwz_transport::protocol::Failure> {
        self.missing.get(&key.operation).cloned()
    }
    pub(crate) fn latch_missing_git(
        &mut self,
        key: &RouteKey,
        failure: gwz_transport::protocol::Failure,
    ) {
        self.missing.entry(key.operation.clone()).or_insert(failure);
    }
}
