//! Private in-process composition for the HTTPS RPC candidate. Delivery consists
//! only of existing envelopes; host placement supplies the same boundary in H2.
use super::{
    https_remote::OpenRpc,
    https_worker::{Client, Input},
    stream_io::BlockingStream,
};
use gwz_transport::{
    protocol::*,
    stream::{Config, MessageEndpoint, Side, Stream},
};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
#[derive(Debug)]
struct OpenFailure {
    failure: Failure,
    anonymous_status: Option<i64>,
}
impl std::fmt::Display for OpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(status) = self.anonymous_status {
            write!(f, "anonymous discovery returned HTTP {status}; ")?;
        }
        write!(f, "HTTPS endpoint request failed: {:?}", self.failure.code)
    }
}
impl std::error::Error for OpenFailure {}
pub(crate) struct LocalRpc {
    client: Client,
    dependency: Result<super::https_operation::Dependency, ErrorCode>,
    runtime: Handle,
    session: String,
    operation: String,
    policy: Option<AuthPolicy>,
    resolved: Arc<Mutex<Option<AuthPolicy>>>,
    first_failure: Arc<Mutex<Option<Failure>>>,
    current: Mutex<Option<CancellationToken>>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}
impl LocalRpc {
    pub(crate) fn new(
        client: Client,
        session: String,
        operation: String,
        policy: Option<AuthPolicy>,
    ) -> Arc<Self> {
        let dependency = client.operation(&operation);
        Arc::new(Self {
            dependency,
            client,
            runtime: Handle::current(),
            session,
            operation,
            policy,
            resolved: Arc::new(Mutex::new(None)),
            first_failure: Arc::new(Mutex::new(None)),
            current: Mutex::new(None),
            tasks: Mutex::new(Vec::new()),
        })
    }
    pub(crate) async fn drain(&self, limit: Duration) -> usize {
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(|e| e.into_inner()));
        let deadline = tokio::time::Instant::now() + limit;
        let mut pending = Vec::new();
        for mut task in tasks {
            if tokio::time::timeout_at(deadline, &mut task).await.is_err() {
                pending.push(task);
            }
        }
        let count = pending.len();
        self.tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(pending);
        count
    }
}
impl OpenRpc for LocalRpc {
    fn open(&self, url: &str, service: GitService) -> io::Result<BlockingStream> {
        if let Err(code) = &self.dependency {
            return Err(io::Error::other(format!(
                "HTTPS operation unavailable: {code:?}"
            )));
        }
        let cancel = CancellationToken::new();
        *self.current.lock().map_err(|_| io::ErrorKind::Other)? = Some(cancel.clone());
        let policy = self
            .policy
            .or(*self.resolved.lock().map_err(|_| io::ErrorKind::Other)?)
            .unwrap_or(AuthPolicy::Anonymous);
        let input = Input {
            destination: url.into(),
            service,
            policy,
            session: self.session.clone(),
            operation: self.operation.clone(),
        };
        let auto = self.policy.is_none();
        let resolved = self.resolved.clone();
        let receipt = self.first_failure.clone();
        let client = self.client.clone();
        static NEXT_SESSION: AtomicI64 = AtomicI64::new(1);
        let session = format!("h1-{}", NEXT_SESSION.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let task=self.runtime.spawn(async move {
            let result = async {
                use super::https_opening::{self, OpeningSession, Outcome};
                let mut input = input.clone();
                if auto && super::https_policy::receive_pack(input.service) { input.policy = AuthPolicy::Gh; }
                let open = https_opening::open_for(&client, &input).map_err(|f| (f, None))?;
                let mut mux = OpeningSession::new(session, "rpc".into(), open, "https-endpoint".into(), "endpoint-account".into()).map_err(|f| (f, None))?;
                let outcome = if auto {
                    let mut gh_input = input.clone(); gh_input.policy = AuthPolicy::Gh;
                    let gh_open = https_opening::open_for(&client, &gh_input).map_err(|f| (f, None))?;
                    mux.prepare_automatic(&client, input, gh_open, gh_input, &cancel).await
                } else { mux.prepare(&client, input, &cancel).await };
                match outcome {
                    Outcome::Ready {prepared, first_failure, ..} => Ok((prepared, mux, first_failure)),
                    Outcome::Failed {failure, first_failure, ..} => Err((failure, first_failure)),
                    Outcome::Rejected(failure) => Err((failure, None)),
                }
            }.await;
            let (prepared, mut mux) = match result {
                Ok((prepared, mux, first)) => { *receipt.lock().unwrap_or_else(|e|e.into_inner()) = first; (prepared, mux) },
                Err((f, first)) => {
                    let anonymous_status = first.as_ref().and_then(|f|f.facts.as_ref()).and_then(|f|f.http_status);
                    *receipt.lock().unwrap_or_else(|e|e.into_inner()) = first;
                    let kind=match f.code {ErrorCode::Authentication|ErrorCode::RepositoryRefused=>io::ErrorKind::PermissionDenied,ErrorCode::Timeout=>io::ErrorKind::TimedOut,ErrorCode::Cancelled=>io::ErrorKind::ConnectionAborted,_=>io::ErrorKind::Other};
                    let _=tx.send(Err(io::Error::new(kind,OpenFailure {failure:f,anonymous_status})));return;
                }
            };
            if auto {*resolved.lock().unwrap_or_else(|e|e.into_inner())=Some(if prepared.opened.facts.method==AuthMethod::Gh {AuthPolicy::Gh}else{AuthPolicy::Anonymous});}
            let mut config=Config::new(mux.session_id(),mux.stream_id(),Side::Initiator);config.profile_version=2;config.io_timeout_ms=prepared.io_timeout_ms();
            let (stream,left)=match Stream::new(config.clone()){Ok(pair)=>pair,Err(e)=>{let _=tx.send(Err(io::Error::other(e)));return;}};
            config.side=Side::Endpoint;
            let (endpoint,right)=match Stream::new(config){Ok(pair)=>pair,Err(e)=>{let _=tx.send(Err(io::Error::other(e)));return;}};
            let left=Arc::new(left);let right=Arc::new(right);
            if tx.send(Ok(BlockingStream::new(stream))).is_err(){return;}
            let _endpoint_owner=endpoint.clone();
            let work=prepared.serve(endpoint,right.clone(),cancel.clone());tokio::pin!(work);
            let start=tokio::time::Instant::now();let mut timer=tokio::time::interval(Duration::from_millis(2));let mut finished=false;
            loop {tokio::select! {
                _=&mut work,if !finished=>{finished=true;},
                _=cancel.cancelled()=>break,
                message=left.next_message()=>match message {Ok(Some(m))=>{match mux.route_initiator(m) {Ok(m)=>{if right.deliver(m).is_err(){break;}},Err(_)=>break}},_=>break},
                message=right.next_message()=>match message {Ok(Some(m))=>{let terminal=matches!(m.kind,MessageKind::Closed|MessageKind::Failed);match mux.route_endpoint(m) {Ok(m)=>{let _=left.deliver(m);},Err(_)=>break}if terminal{break;}},_=>break},
                _=timer.tick()=>{let now=start.elapsed().as_millis() as u64;left.advance(now);right.advance(now);},
            }}
            left.disconnect();right.disconnect();
        });
        let mut tasks = self.tasks.lock().map_err(|_| io::ErrorKind::Other)?;
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
        drop(tasks);
        rx.recv().map_err(|_| io::ErrorKind::BrokenPipe)?
    }
    fn cancel(&self) {
        if let Ok(current) = self.current.lock() {
            if let Some(cancel) = current.as_ref() {
                cancel.cancel();
            }
        }
    }
}
impl Drop for LocalRpc {
    fn drop(&mut self) {
        self.cancel();
    }
}
