//! The words of a member's final `Capacity` failure (adaptive concurrency
//! design §5.3), one text per cause, the rule that picks the cause of a
//! server's refusal, and the `capacity` detail of gwz-transport's
//! `FailureDetail` that carries a [`CapacityReport`] from the endpoint that
//! ends a member to the code that words the error. Pure: it reads a report and
//! writes a message or a detail (and back), so the wording is pinned here once
//! and every endpoint that ends a member with `Capacity` says the same thing.
use super::{hold::HOLD_CAP_MS, notes::site_name};
use gwz_transport::{
    pool::Site,
    protocol::{CapacityCause as WireCause, CapacityDetail, CapacitySignal, FailureDetail},
};

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

impl CapacityCause {
    fn wire(self) -> WireCause {
        match self {
            Self::ServerThrottle => WireCause::ServerThrottle,
            Self::ServerRetryAfter => WireCause::ServerRetryAfter,
            Self::Local => WireCause::Local,
        }
    }
}

/// The status that ended a throttled member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LastSignal {
    Http429,
    Http503,
}

impl LastSignal {
    /// The signal a response status is, if it is one the detail can name: a
    /// 403 that carried `Retry-After` is a throttle (OQ4) with no signal of its
    /// own.
    pub(crate) fn of_status(status: u16) -> Option<Self> {
        match status {
            429 => Some(Self::Http429),
            503 => Some(Self::Http503),
            _ => None,
        }
    }

    fn wire(self) -> CapacitySignal {
        match self {
            Self::Http429 => CapacitySignal::Http429,
            Self::Http503 => CapacitySignal::Http503,
        }
    }

    fn from_wire(signal: CapacitySignal) -> Self {
        match signal {
            CapacitySignal::Http429 => Self::Http429,
            CapacitySignal::Http503 => Self::Http503,
        }
    }
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
    /// `None` for a 403 that carried `Retry-After`.
    pub(crate) last_signal: Option<LastSignal>,
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

    /// The report a failure carries: a final `Capacity` failure whose `capacity`
    /// detail can be worded. A `Capacity` failure without one (the pool's own
    /// limit) carries none.
    pub(crate) fn of_failure(failure: &gwz_transport::protocol::Failure) -> Option<Self> {
        if failure.code != gwz_transport::protocol::ErrorCode::Capacity {
            return None;
        }
        Self::from_detail(failure.detail.as_ref()?.capacity.as_ref()?)
    }

    /// The words the user reads: the message, then the `(attempt N of M)` suffix
    /// of the existing form when the failure reports its attempt. A local wait
    /// is no attempt of the member's and never shows one (§5.3).
    pub(crate) fn words(&self, attempt: Option<(u32, u32)>) -> String {
        let message = self.message();
        match (self, attempt) {
            (Self::Server(_), Some((attempt, attempts))) => {
                format!("{message} (attempt {attempt} of {attempts})")
            }
            _ => message,
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

impl CapacityReport {
    /// The detail of a failure that ended on a local wait: only its cause.
    pub(crate) fn local_failure_detail() -> Box<FailureDetail> {
        Box::new(FailureDetail {
            capacity: Some(Self::Local.detail()),
            ..FailureDetail::default()
        })
    }

    /// The `capacity` detail that carries this report on the wire. The numbers
    /// are made to fit the transport's ranges: a ceiling of at least 1, a
    /// limit between 1 and the ceiling.
    pub(crate) fn detail(&self) -> CapacityDetail {
        let Self::Server(server) = self else {
            return CapacityDetail {
                cause: self.cause().wire(),
                ..CapacityDetail::default()
            };
        };
        let ceiling = i64::from(
            u32::try_from(server.max_per_host)
                .unwrap_or(u32::MAX)
                .max(1),
        );
        CapacityDetail {
            cause: self.cause().wire(),
            scheme: server.site.as_ref().map(|site| site.scheme),
            host: server.site.as_ref().map(|site| site.host.clone()),
            port: server.site.as_ref().map(|site| i64::from(site.port)),
            limit_applied: server
                .limit_applied
                .map(|limit| i64::try_from(limit).unwrap_or(i64::MAX).clamp(1, ceiling)),
            max_per_host: Some(ceiling),
            elapsed_ms: Some(saturate(server.elapsed_ms)),
            last_signal: server.last_signal.map(LastSignal::wire),
            retry_after_ms: server.retry_after_ms.map(saturate),
        }
    }

    /// The report a `capacity` detail carries, or `None` when it carries none
    /// that can be worded: a server cause without its ceiling or its time, a
    /// scheme, host and port that are not all there, or a cause that the
    /// `Retry-After` it carries does not give (the 30 s rule is core's, so the
    /// cause on the wire must agree with it).
    pub(crate) fn from_detail(detail: &CapacityDetail) -> Option<Self> {
        if detail.cause == CapacityCause::Local.wire() {
            return Some(Self::Local);
        }
        let site = match (&detail.scheme, &detail.host, detail.port) {
            (Some(scheme), Some(host), Some(port)) => Some(Site {
                scheme: *scheme,
                host: host.clone(),
                port: u16::try_from(port).ok()?,
            }),
            (None, None, None) => None,
            _ => return None,
        };
        let server = ServerCapacity {
            site,
            limit_applied: match detail.limit_applied {
                Some(limit) => Some(usize::try_from(limit).ok()?),
                None => None,
            },
            max_per_host: usize::try_from(detail.max_per_host?).ok()?,
            elapsed_ms: u64::try_from(detail.elapsed_ms?).ok()?,
            last_signal: detail.last_signal.map(LastSignal::from_wire),
            retry_after_ms: match detail.retry_after_ms {
                Some(wait) => Some(u64::try_from(wait).ok()?),
                None => None,
            },
        };
        (detail.cause == server.cause().wire()).then_some(Self::Server(server))
    }
}

fn saturate(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
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
                    last_signal: Some(LastSignal::Http429),
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

            fn round_trip(report: &CapacityReport) -> Option<CapacityReport> {
                CapacityReport::from_detail(&report.detail())
            }

            #[test]
            fn every_report_round_trips_through_its_wire_detail() {
                for report in [
                    CapacityReport::Server(server(None)),
                    CapacityReport::Server(server(Some(2_000))),
                    CapacityReport::Server(server(Some(120_000))),
                    CapacityReport::Server(ServerCapacity { site: None, ..server(None) }),
                    CapacityReport::Server(ServerCapacity { limit_applied: None, ..server(None) }),
                    CapacityReport::Server(ServerCapacity { last_signal: None, ..server(Some(1_000)) }),
                    CapacityReport::Server(ServerCapacity { last_signal: Some(LastSignal::Http503), ..server(Some(1_000)) }),
                    CapacityReport::Server(ServerCapacity { site: site("::1"), ..server(None) }),
                    CapacityReport::Local,
                ] {
                    assert_eq!(round_trip(&report).as_ref(), Some(&report));
                }
            }

            #[test]
            fn the_detail_names_the_cause_and_only_what_the_cause_has() {
                let throttle = CapacityReport::Server(server(Some(2_000))).detail();
                assert_eq!(throttle.cause, WireCause::ServerThrottle);
                assert_eq!(throttle.scheme, Some(Scheme::Https));
                assert_eq!(throttle.host.as_deref(), Some("github.com"));
                assert_eq!(throttle.port, Some(443));
                assert_eq!(throttle.limit_applied, Some(8));
                assert_eq!(throttle.max_per_host, Some(32));
                assert_eq!(throttle.elapsed_ms, Some(21_000));
                assert_eq!(throttle.last_signal, Some(CapacitySignal::Http429));
                assert_eq!(throttle.retry_after_ms, Some(2_000));
                let long = CapacityReport::Server(server(Some(120_000))).detail();
                assert_eq!(long.cause, WireCause::ServerRetryAfter);
                assert_eq!(long.retry_after_ms, Some(120_000));
                // A redirect target is never named.
                let redirected = CapacityReport::Server(ServerCapacity { site: None, ..server(None) }).detail();
                assert_eq!((redirected.scheme, redirected.host, redirected.port), (None, None, None));
                // Local says nothing but its cause.
                assert_eq!(
                    CapacityReport::Local.detail(),
                    CapacityDetail { cause: WireCause::Local, ..CapacityDetail::default() }
                );
            }

            #[test]
            fn the_numbers_are_made_to_fit_the_transports_ranges() {
                let wild = CapacityReport::Server(ServerCapacity {
                    limit_applied: Some(0),
                    max_per_host: usize::MAX,
                    elapsed_ms: u64::MAX,
                    retry_after_ms: Some(u64::MAX),
                    ..server(None)
                })
                .detail();
                assert_eq!(wild.max_per_host, Some(i64::from(u32::MAX)));
                assert_eq!(wild.limit_applied, Some(1));
                assert_eq!(wild.elapsed_ms, Some(i64::MAX));
                assert_eq!(wild.retry_after_ms, Some(i64::MAX));
                let zero = CapacityReport::Server(ServerCapacity { max_per_host: 0, limit_applied: Some(5), ..server(None) }).detail();
                assert_eq!((zero.max_per_host, zero.limit_applied), (Some(1), Some(1)));
            }

            #[test]
            fn every_detail_a_report_makes_is_one_the_transport_admits() {
                use gwz_transport::protocol::{Effect, Envelope, ErrorCode, Failure, FailureDetail, MessageKind};
                for report in [
                    CapacityReport::Server(server(None)),
                    CapacityReport::Server(server(Some(120_000))),
                    CapacityReport::Server(ServerCapacity { site: None, last_signal: None, limit_applied: None, ..server(None) }),
                    CapacityReport::Local,
                ] {
                    let message = Envelope {
                        version: 2,
                        session_id: "capacity".into(),
                        stream_id: 1,
                        kind: MessageKind::OpenFailed,
                        open_failed: Some(Failure {
                            code: ErrorCode::Capacity,
                            effect: Effect::None,
                            detail: Some(Box::new(FailureDetail {
                                capacity: Some(report.detail()),
                                ..FailureDetail::default()
                            })),
                            ..Failure::default()
                        }),
                        ..Envelope::default()
                    };
                    assert_eq!(gwz_transport::codec::admit(&message), Ok(()), "{report:?}");
                }
            }

            #[test]
            fn a_detail_that_cannot_be_worded_is_no_report() {
                let good = CapacityReport::Server(server(None)).detail();
                for broken in [
                    CapacityDetail { max_per_host: None, ..good.clone() },
                    CapacityDetail { elapsed_ms: None, ..good.clone() },
                    CapacityDetail { elapsed_ms: Some(-1), ..good.clone() },
                    CapacityDetail { port: None, ..good.clone() },
                    CapacityDetail { host: None, ..good.clone() },
                    CapacityDetail { port: Some(70_000), ..good.clone() },
                    // The cause must be the one the Retry-After gives.
                    CapacityDetail { cause: WireCause::ServerRetryAfter, ..good.clone() },
                    CapacityDetail { retry_after_ms: Some(120_000), ..good.clone() },
                ] {
                    assert_eq!(CapacityReport::from_detail(&broken), None, "{broken:?}");
                }
            }

            #[test]
            fn the_words_end_with_the_attempt_for_a_server_cause_and_never_for_a_local_one() {
                let throttle = CapacityReport::Server(server(None));
                assert!(throttle.words(Some((4, 4))).ends_with("--max-per-host (attempt 4 of 4)"));
                assert_eq!(throttle.words(None), throttle.message());
                assert_eq!(CapacityReport::Local.words(Some((2, 4))), CapacityReport::Local.message());
            }

            #[test]
            fn only_a_capacity_failure_with_a_wordable_detail_carries_a_report() {
                use gwz_transport::protocol::{ErrorCode, Failure};
                let with = |code, report: &CapacityReport| Failure {
                    code,
                    detail: Some(Box::new(FailureDetail { capacity: Some(report.detail()), ..FailureDetail::default() })),
                    ..Failure::default()
                };
                let report = CapacityReport::Server(server(None));
                assert_eq!(CapacityReport::of_failure(&with(ErrorCode::Capacity, &report)), Some(report.clone()));
                assert_eq!(CapacityReport::of_failure(&with(ErrorCode::Io, &report)), None);
                assert_eq!(CapacityReport::of_failure(&Failure { code: ErrorCode::Capacity, ..Failure::default() }), None);
            }

            #[test]
            fn only_a_429_and_a_503_are_signals() {
                assert_eq!(LastSignal::of_status(429), Some(LastSignal::Http429));
                assert_eq!(LastSignal::of_status(503), Some(LastSignal::Http503));
                assert_eq!(LastSignal::of_status(403), None);
                assert_eq!(LastSignal::of_status(200), None);
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
