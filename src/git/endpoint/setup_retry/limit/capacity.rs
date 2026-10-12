//! The words of a member's final `Capacity` failure (adaptive concurrency
//! design §5.3), one text per cause, and the rule that picks the cause of a
//! server's refusal. Pure: it reads a report and writes a message, so the
//! wording is pinned here once and every endpoint that ends a member with
//! `Capacity` says the same thing.
//!
//! What is not here: carrying the report to the user. The optional `capacity`
//! detail of §5.3 is a new field of gwz-transport's `FailureDetail`, which
//! needs a protocol change (see the lane report of `limitdocs`); until it
//! exists the failure cannot carry a [`CapacityReport`] from the endpoint to
//! the code that words the error.
use super::{hold::HOLD_CAP_MS, notes::site_name};
use gwz_transport::pool::Site;

/// The fixed number of connection setups gwz runs at once (`agent_job`'s
/// job budget), which a local-capacity message names.
const SETUP_SLOTS: usize = 64;
/// The seconds a local permit wait lasts before it expires: the allocation
/// clock of a member's open.
const LOCAL_WAIT_SECONDS: u64 = 30;

/// Which of the three things happened when a member finished with `Capacity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapacityCause {
    /// The member's own final failure was a 429, or a 503 with `Retry-After`.
    ServerThrottle,
    /// The `Retry-After` of that failure was longer than one command holds a
    /// host (30 s).
    ServerRetryAfter,
    /// A local permit wait expired: the setup slots stayed full.
    Local,
}

/// The status that ended a throttled member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LastSignal {
    Http429,
    Http503,
}

/// What a throttled member knows when it gives up.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ServerCapacity {
    /// The member's configured host and port; `None` when the pool key
    /// differs from it (a cross-host redirect), since a redirect target is
    /// never shown.
    pub(crate) site: Option<Site>,
    /// The per-host connection limit gwz was using when the member gave up,
    /// if it had lowered it.
    pub(crate) limit_applied: Option<usize>,
    /// The ceiling: the resolved `--max-per-host`.
    pub(crate) max_per_host: usize,
    /// How long the member was in the command, in milliseconds.
    pub(crate) elapsed_ms: u64,
    pub(crate) last_signal: LastSignal,
    pub(crate) retry_after_ms: Option<u64>,
}

/// The report of a member that finished with `Capacity`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CapacityReport {
    Server(ServerCapacity),
    Local,
}

impl ServerCapacity {
    /// A `Retry-After` beyond the 30 s one command holds a host is a cause of
    /// its own; 30 s exactly is still a throttle.
    fn cause(&self) -> CapacityCause {
        if self.retry_after_ms.is_some_and(|wait| wait > HOLD_CAP_MS) {
            CapacityCause::ServerRetryAfter
        } else {
            CapacityCause::ServerThrottle
        }
    }

    /// The host as the member configured it, or "the server" when the pool's
    /// key is not the member's own (a redirect target is never shown).
    fn host(&self) -> String {
        self.site
            .as_ref()
            .map_or_else(|| "the server".to_owned(), site_name)
    }
}

impl CapacityReport {
    pub(crate) fn cause(&self) -> CapacityCause {
        match self {
            Self::Server(server) => server.cause(),
            Self::Local => CapacityCause::Local,
        }
    }

    /// The message, after the `member '<id>' at '<path>': ` prefix and before
    /// the `(attempt N of M)` suffix the attempt count adds.
    pub(crate) fn message(&self) -> String {
        let Self::Server(server) = self else {
            return format!(
                "gwz's {SETUP_SLOTS} connection-setup slots stayed full for {LOCAL_WAIT_SECONDS} s (setups that stopped answering hold them until they time out); retry later"
            );
        };
        let host = server.host();
        match (server.cause(), server.retry_after_ms) {
            (CapacityCause::ServerRetryAfter, Some(wait_ms)) => format!(
                "{host} asked to wait {} s, longer than the {} s one command holds a host; retry in {}",
                wait_ms.div_ceil(1_000),
                HOLD_CAP_MS / 1_000,
                wait_in_words(wait_ms)
            ),
            _ => {
                // The ceiling is the resolved --max-per-host; the limit in use
                // is only worth saying when gwz had to lower it.
                let using = server
                    .limit_applied
                    .filter(|limit| *limit < server.max_per_host)
                    .map_or_else(String::new, |limit| {
                        format!(
                            " (using {limit} at a time, ceiling {} from --max-per-host)",
                            server.max_per_host
                        )
                    });
                format!(
                    "{host} kept refusing connections{using}; gave up after {} s; retry later, or lower --max-per-host",
                    server
                        .elapsed_ms
                        .saturating_add(500)
                        .saturating_div(1_000)
                        .max(1)
                )
            }
        }
    }
}

/// How long to wait, in the unit a person reads: whole minutes, then hours,
/// then days, always rounded up so the wait is never understated.
fn wait_in_words(wait_ms: u64) -> String {
    let minutes = wait_ms.div_ceil(60_000);
    if minutes < 60 {
        return format!("{minutes} min");
    }
    let hours = minutes.div_ceil(60);
    if hours < 48 {
        return format!("{hours} h");
    }
    format!("{} days", hours.div_ceil(24))
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;
            use gwz_transport::protocol::Scheme;

            fn site(host: &str) -> Option<Site> {
                Some(Site { scheme: Scheme::Https, host: host.into(), port: 443 })
            }

            fn server(retry_after_ms: Option<u64>) -> ServerCapacity {
                ServerCapacity {
                    site: site("github.com"),
                    limit_applied: Some(8),
                    max_per_host: 32,
                    elapsed_ms: 21_000,
                    last_signal: LastSignal::Http429,
                    retry_after_ms,
                }
            }

            #[test]
            fn a_throttled_member_says_the_host_kept_refusing_and_the_lowered_limit() {
                let report = CapacityReport::Server(server(None));
                assert_eq!(report.cause(), CapacityCause::ServerThrottle);
                assert_eq!(
                    report.message(),
                    "github.com:443 kept refusing connections (using 8 at a time, ceiling 32 from --max-per-host); gave up after 21 s; retry later, or lower --max-per-host"
                );
            }

            #[test]
            fn the_parenthesis_is_left_out_when_the_limit_was_never_lowered() {
                for limit_applied in [None, Some(32)] {
                    let report = CapacityReport::Server(ServerCapacity { limit_applied, ..server(None) });
                    assert_eq!(
                        report.message(),
                        "github.com:443 kept refusing connections; gave up after 21 s; retry later, or lower --max-per-host",
                        "{limit_applied:?}"
                    );
                }
            }

            #[test]
            fn a_retry_after_beyond_what_a_command_holds_is_its_own_cause_and_text() {
                let report = CapacityReport::Server(server(Some(120_000)));
                assert_eq!(report.cause(), CapacityCause::ServerRetryAfter);
                assert_eq!(
                    report.message(),
                    "github.com:443 asked to wait 120 s, longer than the 30 s one command holds a host; retry in 2 min"
                );
            }

            #[test]
            fn thirty_seconds_exactly_is_still_a_throttle() {
                assert_eq!(CapacityReport::Server(server(Some(30_000))).cause(), CapacityCause::ServerThrottle);
                assert_eq!(CapacityReport::Server(server(Some(30_001))).cause(), CapacityCause::ServerRetryAfter);
                assert_eq!(CapacityReport::Server(server(Some(0))).cause(), CapacityCause::ServerThrottle);
            }

            #[test]
            fn a_retry_after_is_never_understated() {
                // 90.5 s is 91 s, and a minute and a half is two minutes.
                let text = CapacityReport::Server(server(Some(90_500))).message();
                assert!(text.contains("asked to wait 91 s,"), "{text}");
                assert!(text.ends_with("retry in 2 min"), "{text}");
                // A part of a minute is a whole one.
                let text = CapacityReport::Server(server(Some(31_000))).message();
                assert!(text.ends_with("retry in 1 min"), "{text}");
                // Long waits are said in hours, then days.
                let text = CapacityReport::Server(server(Some(2 * 3_600_000))).message();
                assert!(text.contains("asked to wait 7200 s,") && text.ends_with("retry in 2 h"), "{text}");
                let text = CapacityReport::Server(server(Some(3 * 86_400_000))).message();
                assert!(text.ends_with("retry in 3 days"), "{text}");
                let text = CapacityReport::Server(server(Some(86_400_000 + 1_000))).message();
                assert!(text.ends_with("retry in 25 h"), "{text}");
            }

            #[test]
            fn the_elapsed_time_is_whole_seconds_and_at_least_one() {
                let at = |elapsed_ms| CapacityReport::Server(ServerCapacity { elapsed_ms, ..server(None) }).message();
                assert!(at(20_499).contains("gave up after 20 s;"));
                assert!(at(20_500).contains("gave up after 21 s;"));
                assert!(at(0).contains("gave up after 1 s;"));
            }

            #[test]
            fn a_host_that_is_not_shown_is_the_server_and_an_ipv6_host_is_bracketed() {
                let redirected = CapacityReport::Server(ServerCapacity { site: None, ..server(None) });
                assert!(redirected.message().starts_with("the server kept refusing connections"));
                let redirected_wait = CapacityReport::Server(ServerCapacity { site: None, ..server(Some(120_000)) });
                assert!(redirected_wait.message().starts_with("the server asked to wait 120 s,"));
                let v6 = CapacityReport::Server(ServerCapacity { site: site("::1"), ..server(None) });
                assert!(v6.message().starts_with("[::1]:443 kept refusing connections"));
            }

            #[test]
            fn a_local_wait_names_the_setup_slots_and_has_no_host() {
                let report = CapacityReport::Local;
                assert_eq!(report.cause(), CapacityCause::Local);
                assert_eq!(
                    report.message(),
                    "gwz's 64 connection-setup slots stayed full for 30 s (setups that stopped answering hold them until they time out); retry later"
                );
                assert_eq!(SETUP_SLOTS, crate::git::endpoint::agent_job::LIMIT);
            }
        }
    }
}
