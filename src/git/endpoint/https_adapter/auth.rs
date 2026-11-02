//! The answer to a challenge (401) to an advertisement, on the challenge's own connection, once per exchange (contract
//! 7.2). The logon session answers `Negotiate` and `NTLM` when the exchange's operation has a native caller (TD1: it is preferred
//! to the helper), and fails closed when the composition's gate is unavailable (contract 7.3); otherwise `Basic` is
//! answered with the credential the operation's record holds for the key (HTTPS model draft r5 3(d) G5), or else with
//! a fresh lookup of the operation's helper (G4), whose credential the exchange records after the authenticated 200.
//! A lookup that returns bytes rejected at the origin is not sent (G7), and an answer refused with another 401 rejects
//! its credential in the record (G6). Anything else, and a challenge that closes its connection, is refused: the
//! exchange belongs to its connection, and no other can continue it.
use super::{
    Binding, closes,
    exchange::{Common, no_body, request, send, stalled},
};
use crate::git::endpoint::{
    https_auth::{
        self, OperationAuth,
        needs::{NeedKey, Recorded},
    },
    https_connection::Connection,
    https_destination::Destination,
    https_worker::native,
};
use gwz_connection::SafetyState;
use gwz_transport::protocol::{self, ErrorCode};
use http_body_util::BodyExt;
use hyper::{Response, body::Incoming};
use std::time::Duration;
use tokio::time::Instant;

/// The most of a challenge's body read and discarded before the answer goes on its connection.
const CHALLENGE_BODY: usize = 64 * 1024;
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
        && let Some(caller) = operation.and_then(OperationAuth::native)
    {
        // Fail closed (contract 7.3): no credential is read and no token is sent while the gate is unavailable.
        if common.safety.state() != SafetyState::Qualified {
            return Err(ErrorCode::Unavailable);
        }
        let until = match common.settings.connect_timeout_ms {
            // The native route needs its fixed deadline D.
            0 => return Err(ErrorCode::UnsupportedOperation),
            ms => Instant::now() + Duration::from_millis(ms),
        };
        drain(challenge, common).await?;
        ready(connection, common).await?;
        let build =
            |authorization: &[u8]| request(destination, service, Some(authorization), no_body());
        let response = native::authenticate_connection(
            caller,
            connection,
            &headers,
            (destination, service),
            &build,
            until,
            &common.retained,
        )
        .await?;
        return Ok(Answered {
            response,
            binding: Binding::LogonSession,
            fresh: None,
        });
    }
    if native::offers_basic(&headers)?
        && let Some(operation) = operation
    {
        drain(challenge, common).await?;
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

/// Reads and discards the challenge's body, so that its connection can carry the answer.
async fn drain(response: Response<Incoming>, common: &Common) -> Result<(), ErrorCode> {
    let mut body = response.into_body();
    let mut read = 0usize;
    while let Some(frame) = stalled(&common.settings, body.frame()).await? {
        let frame = frame.map_err(|_| ErrorCode::Io)?;
        if let Ok(data) = frame.into_data() {
            read = read.saturating_add(data.len());
            if read > CHALLENGE_BODY {
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
