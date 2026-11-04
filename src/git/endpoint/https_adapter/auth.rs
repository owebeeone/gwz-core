//! The answer to a challenge (401) to an advertisement, on the challenge's own connection, once per exchange (contract
//! 7.2). The logon session answers `Negotiate` and `NTLM` when the exchange's operation has a native caller (TD1: it is preferred
//! to the helper), and fails closed when the composition's gate is unavailable (contract 7.3); otherwise `Basic` is
//! answered with the credential the operation's record holds for the key (HTTPS model draft r5 3(d) G5), or else with
//! a fresh lookup of the operation's helper (G4), whose credential the exchange records after the authenticated 200.
//! A lookup that returns bytes rejected at the origin is not sent (G7), and an answer refused with another 401 rejects
//! its credential in the record (G6). Anything else, and a challenge that closes its connection, is refused: the
//! exchange belongs to its connection, and no other can continue it.
//!
//! The re-authentication of a connection-bound key (draft r5 3.1, P5, RA1-RA7) is here too: before the first byte of a
//! POST whose key the logon session authenticated on another connection, the key's own advertisement GET goes on the
//! POST's connection and only the operation's logon session answers it.
use super::{
    Binding, closes,
    exchange::{Common, content_matches, no_body, request, send, stalled},
};
use crate::git::endpoint::{
    https_auth::{
        self, OperationAuth,
        needs::{Evidence, NeedKey, Recorded},
    },
    https_connection::Connection,
    https_destination::Destination,
    https_policy,
    https_worker::native,
};
use gwz_connection::SafetyState;
use gwz_transport::protocol::{self, ErrorCode};
use http_body_util::BodyExt;
use hyper::{Response, body::Incoming};
use std::time::Duration;
use tokio::time::Instant;

/// The most of a challenge's body read and discarded before the answer goes on its connection.
const CHALLENGE_BODY: u64 = 64 * 1024;
/// A re-authentication's 200 body is drained under the larger of this and 1.25 times the advertisement length the
/// record observed for its key (draft r5 3.1, condition 4).
const REAUTHENTICATION_FLOOR: u64 = 8 << 20;
/// A helper's interaction allowance at most (as the endpoint's `gh` path asks).
const HELPER_INTERACTION: Duration = Duration::from_secs(120);
/// The bound that stands for "none" on an allocation.
const UNBOUNDED: Duration = Duration::from_secs(24 * 60 * 60);

/// The answer to a challenge: the final response, the authentication that bound the connection, and the credential a
/// fresh lookup gave, which the exchange records after an authenticated 200 or drops (and so wipes).
pub(super) struct Answered {
    pub(super) response: Response<Incoming>,
    pub(super) binding: Binding,
    pub(super) fresh: Option<https_auth::Secret>,
}

/// Answers `challenge` to the advertisement of `key`. `operation` is the exchange's operation's own authentication (from
/// its context): its credential helper, the logon-session caller its original entry captured, and its credential
/// record. It is none when its context holds one of no provider of ours, and its caller is none for an operation whose
/// entry captured no caller.
pub(super) async fn answer(
    connection: &mut Connection,
    destination: &Destination,
    service: protocol::GitService,
    challenge: Response<Incoming>,
    (operation, key): (Option<&OperationAuth>, &NeedKey),
    common: &Common,
) -> Result<Answered, ErrorCode> {
    if closes(challenge.headers()) {
        return Err(ErrorCode::Authentication);
    }
    let headers = challenge.headers().clone();
    if native::offers_logon_session(&headers)?
        && let Some(operation) = operation
        && let Some(caller) = operation.native()
    {
        // A key the record holds `Rejected` is not answered by the logon session either (G11, G8: "any but
        // `Rejected`"); a tombstone of an `Exhausted` record answers nothing (X4).
        match operation.needs().answer(key)? {
            Recorded::Rejected => return Err(ErrorCode::Authentication),
            Recorded::Tombstone => return Err(ErrorCode::Capacity),
            _ => {}
        }
        // Fail closed (contract 7.3): no credential is read and no token is sent while the gate is unavailable.
        if common.safety.state() != SafetyState::Qualified {
            return Err(ErrorCode::Unavailable);
        }
        let until = match common.settings.connect_timeout_ms {
            // The native route needs its fixed deadline D.
            0 => return Err(ErrorCode::UnsupportedOperation),
            ms => Instant::now() + Duration::from_millis(ms),
        };
        drain(challenge, CHALLENGE_BODY, common).await?;
        ready(connection, common).await?;
        let build =
            |authorization: &[u8]| request(destination, service, Some(authorization), no_body());
        let response = match native::authenticate_connection(
            caller,
            connection,
            &headers,
            (destination, service),
            &build,
            until,
            &common.retained,
        )
        .await
        {
            Ok(response) => response,
            // The logon session's answer is refused (G9): the key is `Rejected` for the rest of the operation, so its
            // next challenge is not answered and its POST sends nothing. A transient failure changes nothing (G10).
            Err(ErrorCode::Authentication) => return Err(operation.needs().refuse(key)),
            Err(code) => return Err(code),
        };
        return Ok(Answered {
            response,
            binding: Binding::LogonSession,
            fresh: None,
        });
    }
    if native::offers_basic(&headers)?
        && let Some(operation) = operation
    {
        drain(challenge, CHALLENGE_BODY, common).await?;
        // A key the record holds is answered with its credential, the header built under the record's lock, and no
        // helper runs (G5); any other is asked of the operation's helper (G4). A rejected key is not answered (G11),
        // nor is one of a record that keeps nothing more (X2).
        let (header, fresh) = match operation.needs().answer(key)? {
            Recorded::Basic(header) => (header, None),
            Recorded::Unrecorded | Recorded::Unretained | Recorded::ConnectionBound { .. } => {
                let secret = lookup(operation.helper(), destination, common).await?;
                // Bytes refused at this origin earlier in the operation are not sent again, for any key (G7).
                operation.needs().vet(key, &secret)?;
                (secret.header(), Some(secret))
            }
            Recorded::Rejected => return Err(ErrorCode::Authentication),
            Recorded::Tombstone => return Err(ErrorCode::Capacity),
        };
        ready(connection, common).await?;
        let answer = request(destination, service, Some(header.as_bytes()), no_body());
        drop(header);
        let response = send(connection, answer?, &common.settings).await?;
        // The credential is refused (G6): every key of the origin holding it is rejected at once, its bytes wiped and
        // its fingerprint kept, and the exchange ends `Authentication`, its connection discarded. A 407 is no verdict
        // on it.
        if response.status() == hyper::StatusCode::UNAUTHORIZED {
            return Err(operation.needs().reject(key, fresh.as_ref()));
        }
        return Ok(Answered {
            response,
            binding: Binding::HelperToken,
            fresh,
        });
    }
    Err(ErrorCode::Authentication)
}

/// The advertisement of the service class of a POST's `service`: the request its re-authentication repeats.
fn advertisement_of(service: protocol::GitService) -> protocol::GitService {
    if https_policy::receive_pack(service) {
        protocol::GitService::ReceivePackAdvertisement
    } else {
        protocol::GitService::UploadPackAdvertisement
    }
}
/// Whether a re-authentication's final answer is the advertisement: a 200 of the service's content type. Anything else
/// is RA7: `classify`'s code, or `Protocol` for a redirect or an interim answer and for another content type.
fn accepted(response: &Response<Incoming>, service: protocol::GitService) -> Result<(), ErrorCode> {
    match https_policy::classify(response.status().as_u16(), service) {
        https_policy::ResponseAction::Success => {}
        https_policy::ResponseAction::Fail(code) => return Err(code),
        https_policy::ResponseAction::Redirect | https_policy::ResponseAction::Interim => {
            return Err(ErrorCode::Protocol);
        }
    }
    if !content_matches(response, service) {
        return Err(ErrorCode::Protocol);
    }
    Ok(())
}

/// Re-authenticates `connection` for the connection-bound `key` before the first byte of its POST of `service` (draft
/// r5 3.1, P5): the key's own advertisement GET, built by `request` as the original was (method, path, query and
/// headers), once, answered only by the operation's own logon session (`operation.native()`, never the helper or a
/// `Basic` record), its 200 body drained under max(8 MiB, 1.25 x `observed`) before the POST is handed to the engine.
/// It returns the authentication that now binds the connection: `LogonSession` when the logon session answered (RA1:
/// the entry becomes `Challenged`), `Anonymous` when a key learned `OnBound` was served unchallenged (RA2a: the POST
/// goes as an unrecorded key's). Every failure ends the exchange before any POST byte (`Effect::None`): a refused
/// logon session (RA4), an unchallenged answer for a key learned `Challenged` (RA2) and a challenge the logon session
/// cannot answer (RA3) reject the key; a transient failure (RA5), a body past its cap (RA6) and any other answer (RA7)
/// leave the entry as it was. The deadline D is minted for the conversation as `answer` mints it.
pub(super) async fn reauthenticate(
    connection: &mut Connection,
    destination: &Destination,
    service: protocol::GitService,
    (operation, key): (&OperationAuth, &NeedKey),
    (observed, evidence): (u64, Evidence),
    common: &Common,
) -> Result<Binding, ErrorCode> {
    let advertisement = advertisement_of(service);
    let cap = REAUTHENTICATION_FLOOR.max(observed.saturating_add(observed / 4));
    ready(connection, common).await?;
    let get = request(destination, advertisement, None, no_body())?;
    // Counted when its GET goes, whatever its outcome (condition 5).
    common.counters.reauthenticated();
    let response = send(connection, get, &common.settings).await?;
    if response.status() != hyper::StatusCode::UNAUTHORIZED {
        accepted(&response, advertisement)?;
        return match evidence {
            Evidence::Challenged => Err(operation.needs().refuse(key)),
            Evidence::OnBound => {
                drain(response, cap, common).await?;
                ready(connection, common).await?;
                Ok(Binding::Anonymous)
            }
        };
    }
    if closes(response.headers()) {
        return Err(ErrorCode::Authentication);
    }
    let headers = response.headers().clone();
    let caller = match operation.native() {
        Some(caller) if native::offers_logon_session(&headers)? => caller,
        _ => return Err(operation.needs().refuse(key)),
    };
    // Fail closed (contract 7.3), as the first answer does.
    if common.safety.state() != SafetyState::Qualified {
        return Err(ErrorCode::Unavailable);
    }
    let until = match common.settings.connect_timeout_ms {
        0 => return Err(ErrorCode::UnsupportedOperation),
        ms => Instant::now() + Duration::from_millis(ms),
    };
    drain(response, CHALLENGE_BODY, common).await?;
    ready(connection, common).await?;
    let build =
        |authorization: &[u8]| request(destination, advertisement, Some(authorization), no_body());
    let answered = match native::authenticate_connection(
        caller,
        connection,
        &headers,
        (destination, advertisement),
        &build,
        until,
        &common.retained,
    )
    .await
    {
        Ok(response) => response,
        Err(ErrorCode::Authentication) => return Err(operation.needs().refuse(key)),
        Err(code) => return Err(code),
    };
    accepted(&answered, advertisement)?;
    operation
        .needs()
        .bound(key, observed, Evidence::Challenged)?;
    drain(answered, cap, common).await?;
    ready(connection, common).await?;
    Ok(Binding::LogonSession)
}

/// Reads and discards a response's body, each frame within the stall bound and at most `cap` bytes in all: a
/// challenge's, so that its connection can carry the answer, or a re-authentication's, before its POST (RA6).
async fn drain(response: Response<Incoming>, cap: u64, common: &Common) -> Result<(), ErrorCode> {
    let mut body = response.into_body();
    let mut read = 0u64;
    while let Some(frame) = stalled(&common.settings, body.frame()).await? {
        let frame = frame.map_err(|_| ErrorCode::Io)?;
        if let Ok(data) = frame.into_data() {
            read = read.saturating_add(data.len() as u64);
            if read > cap {
                return Err(ErrorCode::Protocol);
            }
        }
    }
    Ok(())
}
/// Waits until the connection takes another request.
async fn ready(connection: &mut Connection, common: &Common) -> Result<(), ErrorCode> {
    stalled(&common.settings, connection.sender.ready())
        .await?
        .map_err(|_| ErrorCode::Io)
}
/// The operation's helper's credential for `destination`: one lookup, on the adapter's helper slot and the host's.
async fn lookup(
    config: &https_auth::Config,
    destination: &Destination,
    common: &Common,
) -> Result<https_auth::Secret, ErrorCode> {
    let allocation = match common.settings.allocation_timeout_ms {
        0 => UNBOUNDED,
        ms => Duration::from_millis(ms),
    };
    let started = Instant::now();
    let until = started + allocation;
    let permit = tokio::select! {
        permit = tokio::time::timeout_at(until, common.helpers.clone().acquire_owned()) => {
            permit.map_err(|_| ErrorCode::Timeout)?.map_err(|_| ErrorCode::Cancelled)?
        }
        _ = common.cancel.cancelled() => return Err(ErrorCode::Cancelled),
    };
    let mut remaining = allocation.saturating_sub(started.elapsed());
    let interaction = match common.settings.interaction_timeout_ms {
        0 => HELPER_INTERACTION,
        ms => Duration::from_millis(ms).min(HELPER_INTERACTION),
    };
    https_auth::lookup_until(
        &common.owner,
        config,
        destination,
        &mut remaining,
        interaction,
        &common.cancel,
        https_auth::LookupAdmission {
            until,
            endpoint_slot: Some(permit),
        },
    )
    .await
    .map_err(|error| error.code())
}
