//! What a response tells the limit machine (adaptive concurrency design
//! §3.2, §4.1 and §4.5): a 429, or a 503 or 403 carrying `Retry-After`, is the
//! server saying slow down; any other status is an answer. Reading the headers
//! is here; the machine's judgement is the governor's.
use super::*;
use crate::git::endpoint::setup_retry::Signal;
use hyper::{
    HeaderMap,
    header::{DATE, RETRY_AFTER},
};

/// What the server said about its load in a response, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Said {
    /// The first exchange was answered: the connection is set up.
    Answered,
    /// Slow down, and for how long if the server said (`Retry-After`).
    Throttle { retry_after_ms: Option<u64> },
    /// A status that is neither an answer nor a throttle (a bare 503): the
    /// connection is not set up, and the machine learns nothing.
    Unanswered,
}

/// The cap a `Retry-After` HTTP-date takes when the response has no `Date` to
/// measure it from (§4.5).
const NO_DATE_MS: u64 = 30_000;

/// Classifies a response by its status and headers.
pub(super) fn said(status: u16, headers: &HeaderMap) -> Said {
    let retry_after_ms = retry_after_ms(headers);
    match (status, retry_after_ms) {
        (429, _) => Said::Throttle { retry_after_ms },
        // A 403 is throttle evidence only when it carries `Retry-After`
        // (OQ4); without one it stays what it is today.
        (503 | 403, Some(_)) => Said::Throttle { retry_after_ms },
        (503, None) => Said::Unanswered,
        _ => Said::Answered,
    }
}

/// `Retry-After` in milliseconds: delta-seconds, or an HTTP-date measured from
/// the response's own `Date` so that a skewed local clock cannot stretch it.
/// `None` when absent or unreadable.
pub(super) fn retry_after_ms(headers: &HeaderMap) -> Option<u64> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some(
            value
                .parse::<u64>()
                .map_or(u64::MAX, |seconds| seconds.saturating_mul(1_000)),
        );
    }
    let at = http_date(value)?;
    let date = headers
        .get(DATE)
        .and_then(|date| date.to_str().ok())
        .and_then(|date| http_date(date.trim()));
    Some(date.map_or(NO_DATE_MS, |date| {
        at.saturating_sub(date).saturating_mul(1_000)
    }))
}

/// Seconds since the epoch of an IMF-fixdate, `Sun, 06 Nov 1994 08:49:37 GMT`.
fn http_date(text: &str) -> Option<u64> {
    let (_, rest) = text.split_once(", ")?;
    let mut parts = rest.split(' ');
    let day: u64 = parts.next()?.parse().ok()?;
    let name = parts.next()?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|known| *known == name)? as u64
        + 1;
    let year: u64 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':');
    let (hour, minute, second): (u64, u64, u64) = (
        clock.next()?.parse().ok()?,
        clock.next()?.parse().ok()?,
        clock.next()?.parse().ok()?,
    );
    if parts.next()? != "GMT" || parts.next().is_some() || clock.next().is_some() {
        return None;
    }
    if !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
        || !(1970..=9_999).contains(&year)
    {
        return None;
    }
    // Days from 1970-01-01 to the date (civil-from-days, proleptic Gregorian).
    let (y, m) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = y / 400;
    let year_of_era = y - era * 400;
    let day_of_year = (153 * m + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    // A year below 10,000 keeps every product far below `u64::MAX`; the
    // checked forms keep a later change honest.
    days.checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)
}

impl Client {
    /// The pool's limit machines (the host's observer and the endpoint's
    /// gate).
    pub(crate) fn governor(&self) -> &setup_retry::Governor {
        self.pool.governor()
    }
    /// The pool's clock, which the machines are measured in.
    pub(crate) fn pool_now(&self) -> u64 {
        self.pool.now()
    }
    /// Waits until the hold on `key`'s site, if any, has ended, and the
    /// governor has discarded the site's idle connections as that requires.
    pub(super) async fn wait_for_hold(
        &self,
        key: &Key,
        cancel: &CancellationToken,
    ) -> Result<(), Failure> {
        let governor = self.pool.governor();
        loop {
            let now = self.pool.now();
            if governor.exchange_may_begin(key, now) {
                return Ok(());
            }
            let pause = governor
                .next_deadline(now)
                .map_or(20, |at| at.saturating_sub(now).clamp(5, 100));
            tokio::select! {
                _ = cancel.cancelled() => return Err(failure(ErrorCode::Cancelled)),
                _ = tokio::time::sleep(Duration::from_millis(pause)) => {}
            }
        }
    }
    /// A response arrived on `prepared`'s connection: tells the machines.
    pub(super) fn tell_governor(
        &self,
        prepared: &Prepared,
        key: &Key,
        status: u16,
        headers: &HeaderMap,
    ) {
        let Some(connection) = prepared.lease.as_ref().and_then(HttpLease::pool_connection) else {
            return;
        };
        let governor = self.pool.governor();
        let now = self.pool.now();
        match said(status, headers) {
            Said::Answered => governor.answered(key, connection, now),
            Said::Throttle { retry_after_ms } => {
                governor.refused(
                    key,
                    connection,
                    Signal::Throttle,
                    retry_after_ms,
                    false,
                    now,
                );
            }
            Said::Unanswered => {}
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;
            use hyper::header::HeaderValue;

            fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
                let mut map = HeaderMap::new();
                for (name, value) in pairs {
                    map.insert(*name, HeaderValue::from_str(value).unwrap());
                }
                map
            }

            #[test]
            fn a_429_is_a_throttle_with_or_without_retry_after() {
                assert_eq!(
                    said(429, &headers(&[])),
                    Said::Throttle { retry_after_ms: None }
                );
                assert_eq!(
                    said(429, &headers(&[("retry-after", "7")])),
                    Said::Throttle { retry_after_ms: Some(7_000) }
                );
            }

            #[test]
            fn a_503_or_403_is_a_throttle_only_with_retry_after() {
                let with = headers(&[("retry-after", "2")]);
                let expected = Said::Throttle { retry_after_ms: Some(2_000) };
                assert_eq!(said(503, &with), expected);
                assert_eq!(said(403, &with), expected);
                // A bare 503 is neither an answer nor a throttle; a bare 403
                // is an answer (a private or missing repository, OQ4).
                assert_eq!(said(503, &headers(&[])), Said::Unanswered);
                assert_eq!(said(403, &headers(&[])), Said::Answered);
            }

            #[test]
            fn every_other_status_is_an_answer() {
                for status in [200, 301, 302, 401, 404, 500, 502, 504] {
                    assert_eq!(said(status, &headers(&[("retry-after", "9")])), Said::Answered);
                }
            }

            #[test]
            fn retry_after_is_delta_seconds_or_a_date_measured_from_the_responses_date() {
                assert_eq!(retry_after_ms(&headers(&[("retry-after", " 120 ")])), Some(120_000));
                let date = "Sun, 06 Nov 1994 08:49:37 GMT";
                let later = "Sun, 06 Nov 1994 08:50:07 GMT";
                assert_eq!(
                    retry_after_ms(&headers(&[("retry-after", later), ("date", date)])),
                    Some(30_000)
                );
                // A date already past is no wait.
                assert_eq!(
                    retry_after_ms(&headers(&[("retry-after", date), ("date", later)])),
                    Some(0)
                );
                // With no `Date` to measure from, the cap applies.
                assert_eq!(
                    retry_after_ms(&headers(&[("retry-after", later)])),
                    Some(30_000)
                );
            }

            #[test]
            fn an_http_date_is_seconds_since_the_epoch() {
                assert_eq!(http_date("Sun, 06 Nov 1994 08:49:37 GMT"), Some(784_111_777));
                assert_eq!(http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
                assert_eq!(http_date("Thu, 29 Feb 2024 12:00:00 GMT"), Some(1_709_208_000));
            }

            #[test]
            fn an_unreadable_retry_after_is_none_and_a_huge_one_saturates() {
                for text in [
                    "",
                    "soon",
                    "-5",
                    "1.5",
                    "Sun, 31 Feb 1994 25:00:00 GMT",
                    "Sun, 06 Nov 1994 08:49:37 PST",
                    // Hostile years: no overflow, no panic.
                    "Sun, 06 Nov 1000000000000 08:49:37 GMT",
                    "Sun, 06 Nov 18446744073709551615 08:49:37 GMT",
                    "Sun, 06 Nov 10000 08:49:37 GMT",
                    "Sun, 06 Nov 1969 08:49:37 GMT",
                ] {
                    assert_eq!(retry_after_ms(&headers(&[("retry-after", text)])), None, "{text:?}");
                }
                assert_eq!(retry_after_ms(&HeaderMap::new()), None);
                assert_eq!(
                    retry_after_ms(&headers(&[("retry-after", "99999999999999999999999")])),
                    Some(u64::MAX)
                );
            }
        }
    }
}
