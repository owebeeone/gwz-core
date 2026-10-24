//! The answer to a challenge (401) to an advertisement, on the challenge's own connection, once per exchange (contract
//! 7.2). The logon session answers `Negotiate` and `NTLM` when the adapter has a native caller (TD1: it is preferred
//! to the helper), and fails closed when the composition's gate is unavailable (contract 7.3); otherwise the
//! configured helper answers `Basic`. Anything else, and a challenge that closes its connection, is refused: the
//! exchange belongs to its connection, and no other can continue it.
use super::{
    Binding, closes,
    exchange::{Common, no_body, request, send, stalled},
};
use crate::git::endpoint::{
    https_auth, https_connection::Connection, https_destination::Destination, https_worker::native,
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

/// Answers `challenge`; the final response, and the authentication that bound the connection.
pub(super) async fn answer(
    connection: &mut Connection,
    destination: &Destination,
    service: protocol::GitService,
    challenge: Response<Incoming>,
    common: &Common,
) -> Result<(Response<Incoming>, Binding), ErrorCode> {
    if closes(challenge.headers()) {
        return Err(ErrorCode::Authentication);
    }
    let headers = challenge.headers().clone();
    if native::offers_logon_session(&headers)?
        && let Some(caller) = &common.native
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
        return Ok((response, Binding::LogonSession));
    }
    if native::offers_basic(&headers)?
        && let Some(config) = &common.auth
    {
        drain(challenge, common).await?;
        let secret = lookup(config, destination, common).await?;
        ready(connection, common).await?;
        let header = secret.header();
        let answer = request(destination, service, Some(header.as_bytes()), no_body())?;
        drop(header);
        let response = send(connection, answer, &common.settings).await?;
        return Ok((response, Binding::HelperToken));
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
/// The configured helper's credential for `destination`: one lookup, on the adapter's helper slot and the host's.
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
