//! Private operation helper enablement; the ordinary selection path is unchanged.
use super::*;
type OpenedObserver = Arc<dyn Fn(i64, &Opened) + Send + Sync>;

impl RequestContext {
    pub(crate) fn open(
        &self,
        url: &str,
        service: GitService,
        selected: Option<String>,
        opened: OpenedObserver,
        facts: Arc<dyn Fn(&Facts) + Send + Sync>,
    ) -> io::Result<BlockingStream> {
        self.validate(&self.meta, &self.operation)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "transport scope closed"))?;
        self.session.open(
            &self.meta.request_id,
            &self.operation,
            url,
            service,
            selected
                .as_deref()
                .map(|s| identity(s, &self.meta))
                .unwrap_or_default(),
            true,
            opened,
            facts,
        )
    }
    pub(crate) fn open_with_helpers(
        &self,
        url: &str,
        service: GitService,
        selected: Option<String>,
        helpers_allowed: bool,
        opened: OpenedObserver,
        facts: Arc<dyn Fn(&Facts) + Send + Sync>,
    ) -> io::Result<BlockingStream> {
        if helpers_allowed {
            return self.open(url, service, selected, opened, facts);
        }
        self.validate(&self.meta, &self.operation)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "transport scope closed"))?;
        self.session.open(
            &self.meta.request_id,
            &self.operation,
            url,
            service,
            selected
                .as_deref()
                .map(|s| identity(s, &self.meta))
                .unwrap_or_default(),
            false,
            opened,
            facts,
        )
    }
}
