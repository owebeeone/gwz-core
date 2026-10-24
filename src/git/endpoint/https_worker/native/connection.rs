//! The logon session's conversation on one connection, for the connection host's HTTPS adapter (plan step 1.3c;
//! `git/endpoint/https_adapter/auth.rs`). It is this module's exchange (`exchange.rs`) without the endpoint's pool,
//! routes and budget: every token goes on the connection the challenge came on, whose own final-origin TLS binding the
//! request carries, under the fixed deadline D the adapter gives it, within the eight request rounds of design 8.
//!
//! **Fail closed (contract 7.3).** A caller whose gate gave its supervisor no window has no port: the conversation
//! refuses before any credential is read or any token sent. A conversation dropped or ended before its cleanup is
//! confirmed leaves its record in the adapter's [`NativeRetained`], which the adapter's disposal waits on.
use super::owners::{Finishing, Starting};
use super::*;

/// Native conversations whose cleanup is not yet confirmed. A clone holds the same records.
#[derive(Clone, Default)]
pub(crate) struct NativeRetained(Arc<Mutex<Vec<Box<dyn Probe>>>>);
impl NativeRetained {
    fn retain(&self, probe: Option<Box<dyn Probe>>) {
        if let Some(probe) = probe {
            self.0.lock().unwrap_or_else(|p| p.into_inner()).push(probe);
        }
    }
    /// The records still unconfirmed; a confirmed record is dropped. The checks run outside the lock.
    pub(crate) fn pending(&self) -> usize {
        let records = std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()));
        let left: Vec<_> = records
            .into_iter()
            .filter(|record| !record.confirmed())
            .collect();
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.extend(left);
        state.len()
    }
}

/// Whether the challenge offers the logon session a scheme (`Negotiate` or `NTLM`).
pub(crate) fn offers_logon_session(headers: &HeaderMap) -> Result<bool, ErrorCode> {
    Ok(Offers::parse(headers)?.select().is_some())
}
/// Whether the challenge offers `Basic`.
pub(crate) fn offers_basic(headers: &HeaderMap) -> Result<bool, ErrorCode> {
    Ok(Offers::parse(headers)?.basic)
}

/// What a conversation owns until it ends; dropped early, it cancels and retains each record (`exchange::Guard`).
struct Owner {
    starting: Option<Starting>,
    session: Option<Box<dyn Session>>,
    finishing: Option<Finishing>,
    cancellation: gwz_sspi::Cancellation,
    retained: NativeRetained,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(starting) = self.starting.take() {
            self.retained.retain(Some(Box::new(starting)));
        }
        if let Some(session) = self.session.take() {
            self.retained.retain(session.cancel());
        }
        if let Some(finishing) = self.finishing.take() {
            self.retained.retain(Some(Box::new(finishing)));
        }
    }
}

/// `work` before the deadline D.
async fn before<T>(
    until: Instant,
    work: impl std::future::Future<Output = T>,
) -> Result<T, ErrorCode> {
    tokio::time::timeout_at(until, work)
        .await
        .map_err(|_| ErrorCode::Timeout)
}
/// Reads and discards a continuation's body so that its connection carries the next token; a continuation that
/// closes its connection ends the conversation, which belongs to that connection.
async fn consume(
    response: Response<Incoming>,
    connection: &mut https_connection::Connection,
    until: Instant,
) -> Result<(), ErrorCode> {
    if prepare::closes(response.headers()) {
        return Err(ErrorCode::Authentication);
    }
    let mut body = response.into_body();
    let mut read = 0usize;
    while let Some(frame) = before(until, body.frame()).await? {
        if let Ok(data) = frame.map_err(|_| ErrorCode::Io)?.into_data() {
            read = read.saturating_add(data.len());
            if read > 65536 {
                return Err(ErrorCode::Protocol);
            }
        }
    }
    before(until, connection.sender.ready())
        .await?
        .map_err(|_| ErrorCode::Io)
}

/// Authenticates the logon session to the server that sent `challenge` on `connection`, whose requests `request`
/// builds with a token's `Authorization` value. The final response is the server's 200 (or a 403, 404, 429 or 5xx,
/// which is no verdict on the identity and is the caller's to classify); a refusal of the identity is
/// `Authentication`.
pub(crate) async fn authenticate_connection(
    caller: &NativeCaller,
    connection: &mut https_connection::Connection,
    challenge: &HeaderMap,
    (destination, service): (&Destination, GitService),
    request: &(dyn Fn(&[u8]) -> Result<Request<RequestBody>, ErrorCode> + Sync),
    until: Instant,
    retained: &NativeRetained,
) -> Result<Response<Incoming>, ErrorCode> {
    let offers = Offers::parse(challenge)?;
    let offer = offers.select().ok_or(ErrorCode::Authentication)?;
    offer.initial()?;
    let scheme = offer.scheme;
    let mut history = History::new(scheme);
    // Fail closed: no port, no credential.
    let port = caller.port.as_ref().map_err(|kind| native_code(*kind))?;
    // The binding of this physical connection's final origin handshake, or no attempt at all (design 8).
    let binding = exchange::channel_binding(connection.binding.as_ref())
        .ok_or(ErrorCode::UnsupportedOperation)?;
    let url = destination.request(service);
    let overhead = url[url::Position::BeforePath..].len()
        + destination.authority().len()
        + https_policy::response_type(service).len()
        + b"GET  HTTP/1.1\r\nHost: \r\nAccept: \r\nAuthorization: \r\nTransfer-Encoding: chunked\r\n\r\n".len();
    let available = 65536usize
        .checked_sub(overhead)
        .ok_or(ErrorCode::InvalidRequest)?;
    let limit = raw_limit(available, scheme)?;
    let auth = gwz_sspi::AuthRequest {
        package: if scheme == NativeScheme::Negotiate {
            gwz_sspi::Package::Negotiate
        } else {
            gwz_sspi::Package::Ntlm
        },
        target: gwz_sspi::SecretText::new(&format!("HTTP/{}", destination.host()))
            .map_err(|_| ErrorCode::InvalidRequest)?,
        // The identity is always the logon session's: a helper's credential answers Basic only.
        identity: gwz_sspi::Identity::CurrentLogon,
        channel_binding: binding,
        token_limit: limit,
        digest: None,
    };
    let mut owner = Owner {
        starting: None,
        session: None,
        finishing: None,
        cancellation: gwz_sspi::Cancellation::new(),
        retained: retained.clone(),
    };
    let starting = Starting::new(port.start(auth, until, owner.cancellation.clone()));
    owner.starting = Some(starting.clone());
    before(until, starting).await??;
    owner.session = owner
        .starting
        .take()
        .map(|starting| starting.take_session());
    let mut challenge = None;
    let mut response: Option<Response<Incoming>> = None;
    for _ in 0..NATIVE_ROUNDS {
        let session = owner.session.as_mut().ok_or(ErrorCode::Protocol)?;
        let step = match before(until, session.step(challenge.take())).await? {
            Ok(step) => step,
            Err(error) => {
                owner.retained.retain(error.pending);
                return Err(error.code);
            }
        };
        history.observe(&step)?;
        // Complete with an empty final token may still consume the prior response.
        if !step.payload.as_bytes().is_empty() {
            let header = Header::token(scheme, step.payload.as_bytes())?;
            if let Some(previous) = response.take() {
                consume(previous, connection, until).await?;
            }
            let next = request(header.bytes())?;
            drop(header);
            history.facts.credential_offered = true;
            let sent = before(until, send_request(&mut connection.sender, next)).await?;
            response = Some(sent.map_err(|failed| match failed {
                SendFailure::Sent(error) if error.is_parse() => ErrorCode::Protocol,
                _ => ErrorCode::Io,
            })?);
        }
        let Some(answer) = response.as_ref() else {
            return Err(ErrorCode::Authentication);
        };
        let status = answer.status().as_u16();
        history.reject(status);
        if status == 200 {
            if !history.complete {
                let finals = Offers::parse(answer.headers())?;
                let token = finals
                    .native
                    .iter()
                    .find(|next| next.scheme == scheme)
                    .ok_or(ErrorCode::Protocol)?;
                challenge = Some(decode_challenge(
                    token.token.bytes(),
                    limit.raw_bytes() as usize,
                )?);
                continue;
            }
            history.accept(status)?;
            let session = owner.session.take().ok_or(ErrorCode::Protocol)?;
            let finishing = Finishing::new(session.finish());
            owner.finishing = Some(finishing.clone());
            before(until, finishing).await??;
            owner.finishing = None;
            return response.take().ok_or(ErrorCode::Protocol);
        }
        // No verdict on the identity: the caller classifies it.
        if (400..=599).contains(&status) && status != 401 {
            return response.take().ok_or(ErrorCode::Protocol);
        }
        if status != 401 || history.complete {
            return Err(ErrorCode::Authentication);
        }
        let next = Offers::parse(answer.headers())?;
        let offer = next
            .native
            .iter()
            .find(|next| next.scheme == scheme)
            .ok_or(ErrorCode::Authentication)?;
        // A 401 with no continuation token for the scheme refuses the identity: terminal, with no replay.
        if offer.token.bytes().is_empty() {
            return Err(ErrorCode::Authentication);
        }
        challenge = Some(decode_challenge(
            offer.token.bytes(),
            limit.raw_bytes() as usize,
        )?);
    }
    Err(ErrorCode::Protocol)
}
