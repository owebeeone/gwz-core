use chrono::{DateTime, Local, MappedLocalTime, NaiveDate, NaiveDateTime, TimeZone};
use regex::bytes::Regex;

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::CommitLogEntry;

#[derive(Clone, Debug, Default)]
pub(super) struct CommitLogFilters {
    since: Option<i64>,
    until: Option<i64>,
    author: Option<Regex>,
    grep: Option<Regex>,
    no_merges: bool,
    first_parent: bool,
}

impl CommitLogFilters {
    pub(super) fn from_request(request: &crate::LogRequest) -> ModelResult<Self> {
        let Some(options) = request.options.as_ref() else {
            return Ok(Self::default());
        };
        Ok(Self {
            since: options
                .since
                .as_deref()
                .map(parse_filter_time)
                .transpose()?,
            until: options
                .until
                .as_deref()
                .map(parse_filter_time)
                .transpose()?,
            author: compile("--author", options.author.as_deref())?,
            grep: compile("--grep", options.grep.as_deref())?,
            no_merges: options.no_merges.unwrap_or(false),
            first_parent: options.first_parent.unwrap_or(false),
        })
    }

    pub(super) fn first_parent(&self) -> bool {
        self.first_parent
    }

    pub(super) fn allows(&self, entry: &CommitLogEntry) -> bool {
        let seconds = entry.committer.time.seconds;
        if self.since.is_some_and(|since| seconds < since)
            || self.until.is_some_and(|until| seconds > until)
            || (self.no_merges && entry.parent_ids.len() > 1)
            || self
                .grep
                .as_ref()
                .is_some_and(|pattern| !pattern.is_match(&entry.message))
        {
            return false;
        }
        self.author.as_ref().is_none_or(|pattern| {
            let mut author = entry.author.name.clone();
            author.extend_from_slice(b" <");
            author.extend_from_slice(&entry.author.email);
            author.push(b'>');
            pattern.is_match(&author)
        })
    }
}

fn compile(flag: &str, pattern: Option<&str>) -> ModelResult<Option<Regex>> {
    pattern
        .map(|pattern| {
            Regex::new(pattern).map_err(|error| {
                ModelError::new(
                    ErrorCode::InvalidRequest,
                    format!("invalid {flag} Rust regex '{pattern}': {error}"),
                )
            })
        })
        .transpose()
}

pub(super) fn parse_filter_time(value: &str) -> ModelResult<i64> {
    if let Some(epoch) = value.strip_prefix('@') {
        return epoch.parse::<i64>().map_err(|_| invalid_time(value));
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.timestamp());
    }
    if let Some(parsed) = ["%Y-%m-%dT%H:%M:%S%.f%z", "%Y%m%dT%H%M%S%.f%z"]
        .into_iter()
        .find_map(|format| DateTime::parse_from_str(value, format).ok())
    {
        return Ok(parsed.timestamp());
    }
    let local = if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        date.and_hms_opt(0, 0, 0)
    } else {
        ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"]
            .into_iter()
            .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
    };
    local
        .and_then(|local| unique_timestamp(Local.from_local_datetime(&local)))
        .ok_or_else(|| invalid_time(value))
}

pub(super) fn unique_timestamp<Tz: TimeZone>(local: MappedLocalTime<DateTime<Tz>>) -> Option<i64> {
    local.single().map(|local| local.timestamp())
}

fn invalid_time(value: &str) -> ModelError {
    ModelError::new(
        ErrorCode::InvalidRequest,
        format!(
            "invalid log time '{value}'; expected RFC3339/ISO-8601 (date-only or local offset-less forms allowed) or @<epoch-seconds>; git approxidates are not accepted"
        ),
    )
}
