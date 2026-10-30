//! One exchange's task on the worker's runtime: the smart-HTTP request on its leased connection, the answer to a
//! challenge (`auth.rs`), the response body into the client's buffer, and the settlement that decides whether the
//! connection can serve another exchange.
use super::{
    Binding, HttpsSettings, RepositoryDigest, auth, closes, pipe::Pipe, repository_digest,
};
use crate::git::endpoint::{
    https_auth,
    https_connection::{Connection, RequestBody},
    https_destination::Destination,
    https_policy,
    https_worker::{SendFailure, native::NativeCaller, native::NativeRetained, send_request},
    ssh_adapter::Settle,
};
use bytes::Bytes;
use gwz_connection::{AuthContext, GitService, SharedProcessSafety};
use gwz_transport::{
    pool::Key,
    protocol::{self, Disposition, ErrorCode},
};
use http_body_util::BodyExt;
use hyper::{
    Request, Response,
    body::Incoming,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HOST},
};
use std::{future::Future, io, sync::Arc, time::Duration};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

/// What every exchange of one adapter shares: its budgets, its helper owner and native route, the composition's gate
/// and the cancellation its disposal raises. It holds no credential helper: that is each exchange's operation's own.
pub(super) struct Common {
    pub(super) settings: HttpsSettings,
    pub(super) owner: https_auth::AuthOwner,
    /// The adapter's helper lookups at once (the endpoint's eight); the host's helper slots bound the processes.
    pub(super) helpers: Arc<Semaphore>,
    pub(super) native: Option<NativeCaller>,
    pub(super) safety: SharedProcessSafety,
    /// The native conversations whose cleanup is not yet confirmed; disposal waits for them.
    pub(super) retained: NativeRetained,
    pub(super) cancel: CancellationToken,
}

/// The worker's half of one exchange (`pipe::exchange` makes it with the client's).
pub(crate) struct Work {
    pub(super) key: Key,
    pub(super) service: GitService,
    pub(super) path: String,
    /// A POST's request body, as the client writes it.
    pub(super) body: Option<mpsc::Receiver<io::Result<Bytes>>>,
    /// The context of the operation this exchange serves, whose helper answers its challenges. It is dropped when the
    /// exchange's task ends.
    pub(super) auth: AuthContext,
    /// Which authentication bound the connection when the exchange was given it (the adapter's scope record), and the
    /// digest of the repository path that authentication was for.
    pub(super) prior: Binding,
    pub(super) authenticated: Option<RepositoryDigest>,
    pub(super) pipe: Arc<Pipe>,
    pub(super) settle: Arc<Settle>,
}
impl Work {
    /// The exchange cannot run: its client reads `code`, and its end is `Discarded`.
    pub(super) fn fail(&self, code: ErrorCode) {
        self.pipe.end(Err(code));
        self.settle.end(Disposition::Discarded);
    }
}
/// How an exchange ended: whether its connection can serve another, which authentication bound it and the digest of
/// the repository path the exchange named.
pub(super) struct Ended {
    pub(super) disposition: Disposition,
    pub(super) binding: Binding,
    pub(super) repository: RepositoryDigest,
}

pub(super) fn wire_service(service: GitService) -> protocol::GitService {
    match service {
        GitService::UploadPackAdvertisement => protocol::GitService::UploadPackAdvertisement,
        GitService::UploadPackExchange => protocol::GitService::UploadPackExchange,
        GitService::ReceivePackAdvertisement => protocol::GitService::ReceivePackAdvertisement,
        GitService::ReceivePackExchange => protocol::GitService::ReceivePackExchange,
    }
}
/// The repository the spec names on the key's origin. The path is the URL path; a query or fragment is refused.
pub(super) fn destination(key: &Key, path: &str) -> Result<Destination, ErrorCode> {
    if !path.starts_with('/') || path.contains(['?', '#']) {
        return Err(ErrorCode::InvalidRequest);
    }
    let host = if key.host.contains(':') {
        format!("[{}]", key.host)
    } else {
        key.host.clone()
    };
    Destination::parse(&format!("https://{host}:{}{path}", key.port))
}
/// The smart-HTTP request for `service` at `destination`, with `authorization` when one is offered.
pub(super) fn request(
    destination: &Destination,
    service: protocol::GitService,
    authorization: Option<&[u8]>,
    body: RequestBody,
) -> Result<Request<RequestBody>, ErrorCode> {
    let url = destination.request(service);
    let advertisement = https_policy::advertisement(service);
    let mut request = Request::builder()
        .method(if advertisement { "GET" } else { "POST" })
        .uri(&url[url::Position::BeforePath..])
        .header(HOST, destination.authority())
        .header(ACCEPT, https_policy::response_type(service));
    if !advertisement {
        request = request.header(
            CONTENT_TYPE,
            format!(
                "application/x-{}-request",
                https_policy::service_name(service)
            ),
        );
    }
    if let Some(authorization) = authorization {
        let mut value = hyper::http::HeaderValue::from_bytes(authorization)
            .map_err(|_| ErrorCode::InvalidRequest)?;
        value.set_sensitive(true);
        request = request.header(AUTHORIZATION, value);
    }
    request.body(body).map_err(|_| ErrorCode::InvalidRequest)
}
/// An empty request body, for an advertisement and a challenge's answer.
pub(super) fn no_body() -> RequestBody {
    let (_, rx) = mpsc::channel(1);
    RequestBody { rx }
}
/// Sends `request` on the connection, within the stall bound. A request Hyper never started (the connection had
/// closed) is `Io` like any other failure: the adapter replays nothing (contract 9).
pub(super) async fn send(
    connection: &mut Connection,
    request: Request<RequestBody>,
    settings: &HttpsSettings,
) -> Result<Response<Incoming>, ErrorCode> {
    match stalled(settings, send_request(&mut connection.sender, request)).await? {
        Ok(response) => Ok(response),
        Err(SendFailure::NotStarted) => Err(ErrorCode::Io),
        Err(SendFailure::Sent(error)) if error.is_parse() => Err(ErrorCode::Protocol),
        Err(SendFailure::Sent(_)) => Err(ErrorCode::Io),
    }
}
/// `work` within the exchange's network stall bound.
pub(super) async fn stalled<T>(
    settings: &HttpsSettings,
    work: impl Future<Output = T>,
) -> Result<T, ErrorCode> {
    match settings.io_timeout_ms {
        0 => Ok(work.await),
        ms => tokio::time::timeout(Duration::from_millis(ms), work)
            .await
            .map_err(|_| ErrorCode::Timeout),
    }
}
/// Whether the response's content type is the service's.
pub(super) fn content_matches(
    response: &Response<Incoming>,
    service: protocol::GitService,
) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value
                .trim()
                .eq_ignore_ascii_case(&https_policy::response_type(service))
        })
}

/// Runs `work` on `connection`, which its lease gives the exchange alone. See the module doc.
pub(super) async fn run(
    connection: Arc<AsyncMutex<Connection>>,
    mut work: Work,
    common: Arc<Common>,
) -> Ended {
    let mut guard = connection.lock().await;
    let mut binding = Binding::Anonymous;
    let result = exchange(&mut guard, &mut work, &common, &mut binding).await;
    let disposition = match result {
        Ok(disposition) => disposition,
        Err(code) => {
            work.pipe.end(Err(code));
            Disposition::Discarded
        }
    };
    Ended {
        disposition,
        binding,
        repository: repository_digest(&work.path),
    }
}

async fn exchange(
    connection: &mut Connection,
    work: &mut Work,
    common: &Common,
    binding: &mut Binding,
) -> Result<Disposition, ErrorCode> {
    let service = wire_service(work.service);
    let destination = destination(&work.key, &work.path)?;
    // The helper is the operation's own, from its context; a context of another provider holds none.
    let helper = https_auth::OperationAuth::of(&work.auth).map(https_auth::OperationAuth::helper);
    let body = match work.body.take() {
        Some(rx) => RequestBody { rx },
        None => no_body(),
    };
    // A POST's body is on the wire as the client writes it, so its challenge cannot be answered (contract 7.2). One on
    // a connection a helper token authenticated for this very repository path is sent with the operation's own
    // credential from its first byte, asked of the helper now: the connection serves only the binding that
    // authenticated it, and that server asked for a credential at this path (a token is request-scoped and kept
    // nowhere). A POST to any other path of the connection is anonymous, as a repository the server serves to anyone
    // must stay reachable without a helper.
    let authenticated_here = work.prior == Binding::HelperToken
        && work.authenticated == Some(repository_digest(&work.path));
    let secret = if !https_policy::advertisement(service) && authenticated_here {
        let config = helper.ok_or(ErrorCode::Authentication)?;
        let secret = tokio::select! {
            secret = auth::lookup(config, &destination, common) => secret?,
            _ = work.pipe.interrupted() => return Ok(Disposition::Discarded),
        };
        *binding = Binding::HelperToken;
        Some(secret)
    } else {
        None
    };
    let header = secret.as_ref().map(https_auth::Secret::header);
    let request = request(
        &destination,
        service,
        header.as_ref().map(https_auth::SecretHeader::as_bytes),
        body,
    );
    drop(header);
    drop(secret);
    let request = request?;
    let response = tokio::select! {
        response = send(connection, request, &common.settings) => response?,
        _ = work.pipe.interrupted() => return Ok(Disposition::Discarded),
    };
    let response = match response.status().as_u16() {
        401 if https_policy::advertisement(service) => {
            let (response, bound) = tokio::select! {
                answered = auth::answer(connection, &destination, service, response, helper, common) => answered?,
                _ = work.pipe.interrupted() => return Ok(Disposition::Discarded),
            };
            *binding = bound;
            response
        }
        _ => response,
    };
    match https_policy::classify(response.status().as_u16(), service) {
        https_policy::ResponseAction::Success => {}
        // A redirect is the discovery route's to follow (the endpoint's route table), not one exchange's.
        https_policy::ResponseAction::Redirect => return Err(ErrorCode::UnsupportedOperation),
        https_policy::ResponseAction::Fail(code) => return Err(code),
        https_policy::ResponseAction::Interim => return Err(ErrorCode::Protocol),
    }
    if !content_matches(&response, service) {
        return Err(ErrorCode::Protocol);
    }
    let closing = closes(response.headers());
    let mut body = response.into_body();
    loop {
        if work.pipe.room().await.is_err() {
            // The client ended it with the body unread, or abandoned it: the connection cannot be pooled.
            return Ok(Disposition::Discarded);
        }
        let frame = tokio::select! {
            frame = stalled(&common.settings, body.frame()) => frame?,
            _ = work.pipe.interrupted() => return Ok(Disposition::Discarded),
        };
        match frame {
            None => break,
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    work.pipe.deliver(&data);
                }
            }
            Some(Err(error)) if error.is_parse() => return Err(ErrorCode::Protocol),
            Some(Err(_)) => return Err(ErrorCode::Io),
        }
    }
    drop(body);
    work.pipe.end(Ok(()));
    if !work.pipe.settled().await || closing {
        return Ok(Disposition::Discarded);
    }
    // Hyper confirms the request/response cycle is complete and the connection takes another request.
    let cleanup = Duration::from_millis(common.settings.cleanup_timeout_ms.max(1));
    match tokio::time::timeout(cleanup, connection.sender.ready()).await {
        Ok(Ok(())) => Ok(Disposition::Reusable),
        _ => Ok(Disposition::Discarded),
    }
}
