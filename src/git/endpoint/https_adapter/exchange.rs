//! One exchange's task on the worker's runtime: the smart-HTTP request on its leased connection, the answer to a
//! challenge (`auth.rs`), the response body into the client's buffer, and the settlement that decides whether the
//! connection can serve another exchange.
use super::{Binding, Counters, HttpsSettings, auth, closes, pipe::Pipe};
use crate::git::endpoint::{
    https_auth::{
        self,
        needs::{Evidence, NeedKey, Needs, Recorded},
    },
    https_connection::{Connection, RequestBody},
    https_destination::Destination,
    https_policy,
    https_worker::{SendFailure, native::NativeRetained, send_request},
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

/// What every exchange of one adapter shares: its budgets, its helper owner and retained native records, the composition's
/// gate and the cancellation its disposal raises. It holds no credential helper and no native caller: those are each
/// exchange's operation's own.
pub(super) struct Common {
    pub(super) settings: HttpsSettings,
    pub(super) owner: https_auth::AuthOwner,
    /// The adapter's helper lookups at once (the endpoint's eight); the host's helper slots bound the processes.
    pub(super) helpers: Arc<Semaphore>,
    pub(super) safety: SharedProcessSafety,
    /// The native conversations whose cleanup is not yet confirmed; disposal waits for them.
    pub(super) retained: NativeRetained,
    pub(super) cancel: CancellationToken,
    /// The factory's shared counters, which every re-authentication counts into (draft r5 3.1, Counter).
    pub(super) counters: Counters,
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
    /// Which authentication bound the connection when the exchange was given it (the adapter's scope record): a key
    /// served unchallenged on a logon-session connection is learned `OnBound` (draft r5 3.1, G2).
    pub(super) prior: Binding,
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
/// How an exchange ended: whether its connection can serve another, and which authentication bound it (draft 3(e) W1:
/// every exchange that sent a credential, the record's included, or completed a native conversation says so).
pub(super) struct Ended {
    pub(super) disposition: Disposition,
    pub(super) binding: Binding,
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
    }
}

/// What a POST of `key` carries from its first byte (draft r5 3(d)): the record's credential, its header built under
/// the record's lock, for a key learned `Basic` (P1); none for a key never learned (P3), sent anonymously; none for a
/// connection-bound one, which is sent on a logon-session connection as it is (P4) and re-authenticates any other first
/// (P5). A key the record rejected sends nothing (P7), nor does one it could not keep (P8, X3). A 401 to a POST that
/// carried the record's credential rejects it (P2).
fn post_header(needs: &Needs, key: &NeedKey) -> Result<Post, ErrorCode> {
    match needs.answer(key)? {
        Recorded::Basic(header) => Ok(Post {
            header: Some(header),
            learned: None,
        }),
        Recorded::Unrecorded => Ok(Post::default()),
        Recorded::ConnectionBound { observed, evidence } => Ok(Post {
            header: None,
            learned: Some((observed, evidence)),
        }),
        Recorded::Rejected => Err(ErrorCode::Authentication),
        Recorded::Unretained | Recorded::Tombstone => Err(ErrorCode::Capacity),
    }
}
/// What a POST carries from its first byte, and, for a connection-bound key, the advertisement length its entry observed
/// and how it was learned, which its re-authentication needs.
#[derive(Default)]
struct Post {
    header: Option<https_auth::SecretHeader>,
    learned: Option<(u64, Evidence)>,
}

async fn exchange(
    connection: &mut Connection,
    work: &mut Work,
    common: &Common,
    binding: &mut Binding,
) -> Result<Disposition, ErrorCode> {
    let service = wire_service(work.service);
    let destination = destination(&work.key, &work.path)?;
    // The helper, the logon-session caller and the credential record are the operation's own, from its context; a
    // context of another provider holds none of them. The adapter holds no caller and no credential of its own.
    let operation = https_auth::OperationAuth::of(&work.auth);
    // The exchange's repository and service class in the operation's record (draft r5 3(a)).
    let key = NeedKey::new(&work.key, &destination, service);
    let body = match work.body.take() {
        Some(rx) => RequestBody { rx },
        None => no_body(),
    };
    // A POST's body is on the wire as the client writes it, so its challenge cannot be answered (contract 7.2). A POST
    // of a key an advertisement of this operation learned carries the credential the operation's record holds from its
    // first byte, on whichever connection the pool leased; one of a key never learned goes out anonymously, as a
    // repository the server serves to anyone must stay reachable without a helper. No helper runs for a POST.
    let Post { header, learned } = match operation {
        Some(operation) if !https_policy::advertisement(service) => {
            post_header(operation.needs(), &key)?
        }
        _ => Post::default(),
    };
    // The operation whose record's credential the request carries, if it carries one.
    let carried = operation.filter(|_| header.is_some());
    if carried.is_some() {
        *binding = Binding::HelperToken;
    }
    // A connection-bound key's POST on a connection the logon session has not authenticated re-authenticates it before
    // its first byte (draft r5 3.1, P5); on one it has, the POST goes as it is (P4).
    if let (Some(operation), Some(learned)) = (operation, learned)
        && work.prior != Binding::LogonSession
    {
        *binding = tokio::select! {
            bound = auth::reauthenticate(connection, &destination, service, (operation, &key), learned, common) => bound?,
            _ = work.pipe.interrupted() => return Ok(Disposition::Discarded),
        };
    }
    // The POST rides the logon session's authentication of its connection (P4, RA1): a 401 to it rejects the key (P6).
    // After RA2a it goes as an unrecorded key's, and a 401 to it changes nothing.
    let rides = learned.is_some()
        && (work.prior == Binding::LogonSession || *binding == Binding::LogonSession);
    let request = request(
        &destination,
        service,
        header.as_ref().map(https_auth::SecretHeader::as_bytes),
        body,
    );
    drop(header);
    let request = request?;
    // From here the exchange's own request may reach the server (contract 9, A7); a re-authentication did not move it.
    work.pipe.sending();
    let response = tokio::select! {
        response = send(connection, request, &common.settings) => response?,
        _ = work.pipe.interrupted() => return Ok(Disposition::Discarded),
    };
    // A fresh lookup's credential belongs to this exchange until the record keeps it or it is dropped and wiped.
    let mut fresh = None;
    let challenged = response.status() == hyper::StatusCode::UNAUTHORIZED;
    let response = match (response.status().as_u16(), carried) {
        (401, _) if https_policy::advertisement(service) => {
            // An `Exhausted` record (draft r5 X2, X4; a poisoned lock makes it so here) answers nothing and starts no
            // lookup, and the logon session does not answer for it either: the challenged advertisement ends with the
            // budget code before any byte of an answer.
            if operation.is_some_and(|operation| operation.needs().exhausted()) {
                return Err(ErrorCode::Capacity);
            }
            let answered = tokio::select! {
                answered = auth::answer(connection, &destination, service, response, (operation, &key), common) => answered?,
                _ = work.pipe.interrupted() => return Ok(Disposition::Discarded),
            };
            *binding = answered.binding;
            fresh = answered.fresh;
            answered.response
        }
        // The POST's credential is refused (P2): it is rejected at once for every key of the origin holding it, and
        // the exchange ends `Authentication`, its connection discarded. Its body was on the wire: it is not replayed.
        (401, Some(operation)) => return Err(operation.needs().reject(&key, None)),
        (401, None) if rides => {
            return Err(operation.map_or(ErrorCode::Authentication, |operation| {
                operation.needs().refuse(&key)
            }));
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
    // The key and the credential a fresh lookup answered its challenge with are recorded after the authenticated 200
    // and before any byte of the body reaches the client (draft r5 3(h), G4): a client that read one finds the record,
    // so the POST it opens next carries the credential on any connection. An answer from the record (G5) is there. A key
    // past one of the record's bounds is not kept (G14): the advertisement ends here with the budget code, before a body
    // byte, so no POST follows it.
    if let (Some(secret), Some(operation)) = (fresh, operation) {
        operation.needs().record(&key, secret)?;
    }
    // The connection-bound learning (draft r5 3.1, Learning), at the same point: the logon session answered the
    // challenge (G8, `Challenged`), or the connection was the logon session's when the exchange was given it and the
    // advertisement was served unchallenged (G2, `OnBound`, which proves nothing about the key's need). A `Basic` answer
    // records `Basic` even on such a connection. Then the exchange says its connection is bound when the connection's
    // scope after it is `LogonSession` (W3: the record only rises), before any byte of the body reaches the client.
    let bound = work.prior == Binding::LogonSession || *binding == Binding::LogonSession;
    let observing = match operation {
        Some(operation) if https_policy::advertisement(service) => {
            let evidence = if *binding == Binding::LogonSession {
                Some(Evidence::Challenged)
            } else if bound && !challenged {
                Some(Evidence::OnBound)
            } else {
                None
            };
            if let Some(evidence) = evidence {
                operation.needs().bound(&key, 0, evidence)?;
            }
            Some(operation).filter(|_| bound)
        }
        _ => None,
    };
    if bound {
        work.pipe.bind();
    }
    let closing = closes(response.headers());
    let mut body = response.into_body();
    // The advertisement's length, which a connection-bound key's entry keeps (3.1, condition 4).
    let mut delivered = 0u64;
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
                    delivered = delivered.saturating_add(data.len() as u64);
                    work.pipe.deliver(&data);
                }
            }
            Some(Err(error)) if error.is_parse() => return Err(ErrorCode::Protocol),
            Some(Err(_)) => return Err(ErrorCode::Io),
        }
    }
    drop(body);
    // Written before the exchange's end is visible: a client that read the advertisement finds its length.
    if let Some(operation) = observing {
        operation.needs().observe(&key, delivered);
    }
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
