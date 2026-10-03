//! Git HTTP RPC boundary: body completion is the first nonempty read, not flush.
use super::https_policy;
use super::stream_io::BlockingStream;
use git2::{
    RemoteCallbacks,
    transport::{Service, SmartSubtransport, SmartSubtransportStream},
};
use gwz_transport::protocol::GitService;
use std::{
    io::{self, Read, Write},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

/// Stable, scheme-specific marker consumed by the existing private-member
/// suppression boundary.  The response body and remote URL never cross that
/// boundary.
pub(crate) const REPOSITORY_REFUSED: &str = "GWZ HTTPS repository access refused";

pub(crate) trait HalfClose: Read + Write {
    fn end_write(&self) -> io::Result<()>;
    fn finish(&self) -> io::Result<()>;
    fn cancel(&self);
    fn retained_failure(&self) -> Option<gwz_transport::protocol::Failure> {
        None
    }
    fn retained_failure_facts(&self) -> Option<gwz_transport::protocol::Facts> {
        None
    }
}
impl HalfClose for BlockingStream {
    fn retained_failure(&self) -> Option<gwz_transport::protocol::Failure> {
        BlockingStream::retained_failure(self)
    }
    fn retained_failure_facts(&self) -> Option<gwz_transport::protocol::Facts> {
        BlockingStream::retained_failure_facts(self)
    }
    fn end_write(&self) -> io::Result<()> {
        BlockingStream::end_write(self)
    }
    fn finish(&self) -> io::Result<()> {
        self.close().map(|_| ())
    }
    fn cancel(&self) {
        BlockingStream::cancel(self);
    }
}
pub(crate) struct RpcIo<S: HalfClose> {
    stream: S,
    advertisement: bool,
    report: Option<Arc<dyn Fn(gwz_transport::protocol::Failure, GitService) + Send + Sync>>,
    reading: bool,
    done: bool,
    failed: bool,
    alive: Arc<AtomicBool>,
}
impl<S: HalfClose> RpcIo<S> {
    pub(crate) fn new(stream: S, advertisement: bool) -> Self {
        Self {
            stream,
            advertisement,
            report: None,
            reading: false,
            done: false,
            failed: false,
            alive: Arc::new(AtomicBool::new(true)),
        }
    }
}
impl<S: HalfClose> RpcIo<S> {
    fn failure(&self, error: io::Error) -> io::Error {
        let Some(mut failure) = self.stream.retained_failure() else {
            return error;
        };
        // Closed carries its facts beside Failure. Enrich only this derived
        // callback value; the stream's first admitted Failure stays unchanged.
        if failure.facts.is_none() {
            failure.facts = self.stream.retained_failure_facts();
        }
        let service = if self.advertisement {
            GitService::UploadPackAdvertisement
        } else {
            GitService::UploadPackExchange
        };
        if let Some(report) = &self.report {
            report(failure.clone(), service);
        }
        let message = crate::transport_host::HttpsOpenFailure {
            failure,
            anonymous: None,
            attempts: None,
            service: Some(service),
            helpers_disabled: false,
            cli_hint: true,
        };
        io::Error::new(error.kind(), message)
    }
}
impl<S: HalfClose> Read for RpcIo<S> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.done {
            return Ok(0);
        }
        if !self.reading {
            if let Err(error) = self.stream.end_write() {
                self.failed = true;
                return Err(self.failure(error));
            }
            self.reading = true;
        }
        let count = match self.stream.read(output) {
            Ok(count) => count,
            Err(error) => {
                self.failed = true;
                return Err(self.failure(error));
            }
        };
        if count == 0 {
            if let Err(error) = self.stream.finish() {
                self.failed = true;
                return Err(self.failure(error));
            }
            self.done = true;
            self.alive.store(false, Ordering::Release);
        }
        Ok(count)
    }
}
impl<S: HalfClose> Write for RpcIo<S> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if self.reading || self.done || (self.advertisement && !input.is_empty()) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.stream.write(input)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}
impl<S: HalfClose> Drop for RpcIo<S> {
    fn drop(&mut self) {
        if !self.done {
            // Git may stop reading an advertisement after its packet flush,
            // before requesting stream EOF. Close discards only unread logical
            // response bytes; the endpoint still drains the HTTP body and
            // independently proves the physical connection reusable.
            if !self.advertisement || !self.reading || self.failed || self.stream.finish().is_err()
            {
                self.stream.cancel();
            }
        }
        self.alive.store(false, Ordering::Release);
    }
}
pub(crate) trait OpenRpc: Send + Sync + 'static {
    fn open(&self, url: &str, service: GitService) -> io::Result<BlockingStream>;
    fn cancel(&self);
    fn report_failure(&self, _failure: gwz_transport::protocol::Failure, _service: GitService) {}
}
struct Remote {
    endpoint: Arc<dyn OpenRpc>,
    active: Mutex<Weak<AtomicBool>>,
}
pub(crate) fn install<'a>(callbacks: &mut RemoteCallbacks<'a>, endpoint: Arc<dyn OpenRpc>) {
    callbacks.smart_transport(true, move |_| {
        Ok(Remote {
            endpoint: endpoint.clone(),
            active: Mutex::new(Weak::new()),
        })
    });
}
impl SmartSubtransport for Remote {
    fn action(
        &self,
        url: &str,
        service: Service,
    ) -> Result<Box<dyn SmartSubtransportStream>, git2::Error> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| git2::Error::from_str("HTTP RPC ownership failed"))?;
        if active
            .upgrade()
            .is_some_and(|live| live.load(Ordering::Acquire))
        {
            return Err(git2::Error::from_str(
                "HTTP RPC must finish before another action",
            ));
        }
        let (service, advertisement) = match service {
            Service::UploadPackLs => (GitService::UploadPackAdvertisement, true),
            Service::UploadPack => (GitService::UploadPackExchange, false),
            Service::ReceivePackLs => (GitService::ReceivePackAdvertisement, true),
            Service::ReceivePack => (GitService::ReceivePackExchange, false),
        };
        let stream = self
            .endpoint
            .open(url, service)
            .map_err(|error| map_open_error(error, service))?;
        let mut rpc = RpcIo::new(stream, advertisement);
        let endpoint = self.endpoint.clone();
        rpc.report = Some(Arc::new(move |failure, _| {
            endpoint.report_failure(failure, service)
        }));
        *active = Arc::downgrade(&rpc.alive);
        Ok(Box::new(rpc))
    }
    fn close(&self) -> Result<(), git2::Error> {
        self.endpoint.cancel();
        Ok(())
    }
}

fn map_open_error(error: io::Error, service: GitService) -> git2::Error {
    let Some(failure) = error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<crate::transport_host::HttpsOpenFailure>())
    else {
        return git2::Error::new(
            git2::ErrorCode::GenericError,
            git2::ErrorClass::Http,
            "HTTPS endpoint request failed",
        );
    };

    if is_repository_refused(service, failure) {
        return git2::Error::new(
            git2::ErrorCode::NotFound,
            git2::ErrorClass::Http,
            REPOSITORY_REFUSED,
        );
    }

    // Accepted helper outcomes preserve native clone/private-member classification.
    // TR1.6 §11 permits suppression for the named Authentication outcomes;
    // rejected credentials, missing Git and helper bounds remain visible.
    let code = if failure.helper_timeout() {
        git2::ErrorCode::Timeout
    } else if failure.failure.code == gwz_transport::protocol::ErrorCode::Unavailable
        && failure
            .failure
            .facts
            .as_ref()
            .is_some_and(|f| f.method == gwz_transport::protocol::AuthMethod::Gh)
    {
        git2::ErrorCode::NotFound
    } else if failure.failure.code == gwz_transport::protocol::ErrorCode::Authentication
        && !failure.failure.facts.as_ref().is_some_and(|f| {
            f.method == gwz_transport::protocol::AuthMethod::Sspi || f.authenticated == Some(false)
        })
        && !failure
            .failure
            .detail
            .as_ref()
            .and_then(|d| d.schemes.as_ref())
            .is_some_and(|schemes| schemes.iter().any(|s| s.eq_ignore_ascii_case("Negotiate")))
    {
        git2::ErrorCode::Auth
    } else {
        git2::ErrorCode::GenericError
    };
    git2::Error::new(code, git2::ErrorClass::Http, failure.reason())
}

fn is_repository_refused(
    service: GitService,
    failure: &crate::transport_host::HttpsOpenFailure,
) -> bool {
    if !https_policy::advertisement(service)
        || failure.failure.code != gwz_transport::protocol::ErrorCode::RepositoryRefused
        || failure.failure.effect != gwz_transport::protocol::Effect::None
    {
        return false;
    }
    let Some(facts) = failure.failure.facts.as_ref() else {
        return false;
    };
    facts.authenticated.is_none()
        && matches!(facts.http_status, Some(403 | 404))
        && matches!(
            facts.method,
            gwz_transport::protocol::AuthMethod::None | gwz_transport::protocol::AuthMethod::Gh
        )
}
cfg_if::cfg_if! { if #[cfg(test)] { #[path="https_remote_tests.rs"] mod tests; } }
