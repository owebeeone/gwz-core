//! Author, committer and tagger identity, and Git's date forms.
//!
//! Resolved the way Git does: the `GIT_*` variables first, then the
//! `author.*` / `committer.*` keys, then `user.*`. Anything native cannot
//! reproduce exactly is refused with a reason and the Git CLI decides.

use std::ffi::OsStr;

use super::environment::GitEnvironment;

/// Which signature is being resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Author,
    Committer,
}

impl Role {
    fn env(self, field: &str) -> String {
        let prefix = match self {
            Self::Author => "GIT_AUTHOR_",
            Self::Committer => "GIT_COMMITTER_",
        };
        format!("{prefix}{field}")
    }

    fn config(self, field: &str) -> String {
        let section = match self {
            Self::Author => "author",
            Self::Committer => "committer",
        };
        format!("{section}.{field}")
    }
}

/// A resolved signature: name, email, seconds since the epoch, offset in
/// minutes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Ident {
    pub(super) name: String,
    pub(super) email: String,
    pub(super) seconds: Option<i64>,
    pub(super) offset_minutes: i32,
}

/// Why native does not resolve an identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum IdentRefusal {
    /// Name or email is not given by an explicit source, so Git would guess
    /// from the account and host, or fail.
    NotExplicit(&'static str),
    /// A value Git cleans or rejects in ways native does not copy.
    Unusual(&'static str),
    /// A date form native does not parse.
    Date,
}

/// Looks up one configuration string; `Ok(None)` when unset.
pub(super) trait ConfigStrings {
    fn string(&self, key: &str) -> Result<Option<String>, IdentRefusal>;
}

/// Git strips these from both ends of a name or email, and drops `<`, `>` and
/// newlines inside; libgit2 rejects the inner ones. Native accepts a value only
/// when none of that applies.
fn is_plain(value: &str) -> bool {
    const CRUD: &str = ".,:;<>\"\\'";
    let crud = |c: char| c <= ' ' || CRUD.contains(c);
    !value.is_empty()
        && !value.starts_with(crud)
        && !value.ends_with(crud)
        && !value.contains(['<', '>', '\n', '\0'])
}

fn env_string(
    env: &GitEnvironment,
    name: &str,
    what: &'static str,
) -> Result<Option<String>, IdentRefusal> {
    match env.get(name) {
        None => Ok(None),
        Some(value) => value
            .to_str()
            .map(|text| Some(text.to_owned()))
            .ok_or(IdentRefusal::Unusual(what)),
    }
}

fn field(
    env: &GitEnvironment,
    config: &dyn ConfigStrings,
    role: Role,
    name: &'static str,
    what: &'static str,
) -> Result<String, IdentRefusal> {
    let upper = name.to_ascii_uppercase();
    let lower = name.to_ascii_lowercase();
    let value = match env_string(env, &role.env(&upper), what)? {
        Some(value) => value,
        None => match config.string(&role.config(&lower))? {
            Some(value) => value,
            None => config
                .string(&format!("user.{lower}"))?
                .ok_or(IdentRefusal::NotExplicit(what))?,
        },
    };
    if is_plain(&value) {
        Ok(value)
    } else {
        Err(IdentRefusal::Unusual(what))
    }
}

/// Resolve one identity; the date is `None` for "now".
pub(super) fn resolve(
    env: &GitEnvironment,
    config: &dyn ConfigStrings,
    role: Role,
) -> Result<Ident, IdentRefusal> {
    let name = field(env, config, role, "name", "name")?;
    let email = field(env, config, role, "email", "email")?;
    let (seconds, offset_minutes) = match env.get(&role.env("DATE")) {
        None => (None, 0),
        Some(raw) => {
            let (seconds, offset) = parse_date(raw).ok_or(IdentRefusal::Date)?;
            (Some(seconds), offset)
        }
    };
    Ok(Ident {
        name,
        email,
        seconds,
        offset_minutes,
    })
}

fn digits(text: &str, min: usize, max: usize) -> Option<i64> {
    if text.len() < min || text.len() > max || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Days from 1970-01-01 to the given civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
    }
}

/// `+hhmm`, `-hhmm`, `+hh:mm`, `-hh:mm` or `Z`, as minutes east of UTC.
fn parse_offset(text: &str) -> Option<i32> {
    if text == "Z" {
        return Some(0);
    }
    let sign = match text.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest = &text[1..];
    let (hours, minutes) = match rest.len() {
        4 => (&rest[..2], &rest[2..]),
        5 if rest.as_bytes()[2] == b':' => (&rest[..2], &rest[3..]),
        _ => return None,
    };
    let hours = digits(hours, 2, 2)?;
    let minutes = digits(minutes, 2, 2)?;
    if hours > 14 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 60 + minutes) as i32)
}

/// The two date forms native reproduces: Git's raw `[@]<seconds> <+-hhmm>`
/// and ISO 8601 `<date>T<time> <offset>`. Every other form is Git's to read.
pub(super) fn parse_date(raw: &OsStr) -> Option<(i64, i32)> {
    let text = raw.to_str()?;
    let (stamp, zone) = text.split_once(' ')?;
    let numeric = match stamp.strip_prefix('@') {
        Some(seconds) => digits(seconds, 1, 12),
        None => digits(stamp, 9, 12),
    };
    if let Some(seconds) = numeric {
        if zone == "Z" {
            return None;
        }
        return Some((seconds, parse_offset(zone)?));
    }
    let (date, time) = stamp.split_once('T')?;
    iso(date, time, zone)
}

fn iso(date: &str, time: &str, zone: &str) -> Option<(i64, i32)> {
    let mut parts = date.split('-');
    let year = digits(parts.next()?, 4, 4)?;
    let month = digits(parts.next()?, 2, 2)?;
    let day = digits(parts.next()?, 2, 2)?;
    if parts.next().is_some() {
        return None;
    }
    let mut clock = time.split(':');
    let hour = digits(clock.next()?, 2, 2)?;
    let minute = digits(clock.next()?, 2, 2)?;
    let second = digits(clock.next()?, 2, 2)?;
    if clock.next().is_some() {
        return None;
    }
    if !(1970..=2099).contains(&year)
        || !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let offset = parse_offset(zone)?;
    let local = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    Some((local - i64::from(offset) * 60, offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Map(BTreeMap<&'static str, &'static str>);
    impl ConfigStrings for Map {
        fn string(&self, key: &str) -> Result<Option<String>, IdentRefusal> {
            Ok(self.0.get(key).map(|v| (*v).to_owned()))
        }
    }

    fn map(pairs: &[(&'static str, &'static str)]) -> Map {
        Map(pairs.iter().copied().collect())
    }

    #[test]
    fn environment_beats_role_keys_beat_user_keys() {
        let config = map(&[
            ("user.name", "U"),
            ("user.email", "u@x"),
            ("committer.name", "C"),
            ("author.email", "a@x"),
        ]);
        let env = GitEnvironment::from_vars([("GIT_AUTHOR_NAME", "E")]);
        let author = resolve(&env, &config, Role::Author).unwrap();
        assert_eq!((author.name.as_str(), author.email.as_str()), ("E", "a@x"));
        let committer = resolve(&env, &config, Role::Committer).unwrap();
        assert_eq!(
            (committer.name.as_str(), committer.email.as_str()),
            ("C", "u@x")
        );
    }

    #[test]
    fn missing_or_unusual_identity_is_refused() {
        let env = GitEnvironment::none();
        assert_eq!(
            resolve(&env, &map(&[("user.name", "U")]), Role::Author),
            Err(IdentRefusal::NotExplicit("email"))
        );
        for bad in ["", " lead", "trail ", "Jr.", "a<b", "a\nb", "<x>"] {
            let env = GitEnvironment::from_vars([("GIT_AUTHOR_NAME", bad)]);
            let result = resolve(&env, &map(&[("user.email", "u@x")]), Role::Author);
            assert!(
                matches!(result, Err(IdentRefusal::Unusual("name"))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn dates_in_the_two_supported_forms_parse() {
        assert_eq!(
            parse_date(OsStr::new("2001-02-03T04:05:06 +0530")),
            Some((981_153_306, 330))
        );
        assert_eq!(
            parse_date(OsStr::new("2002-03-04T05:06:07 -07:00")),
            Some((1_015_243_567, -420))
        );
        assert_eq!(
            parse_date(OsStr::new("2001-02-03T04:05:06 Z")),
            Some((981_173_106, 0))
        );
        assert_eq!(
            parse_date(OsStr::new("1015243567 -0700")),
            Some((1_015_243_567, -420))
        );
        assert_eq!(
            parse_date(OsStr::new("@1015243567 +0000")),
            Some((1_015_243_567, 0))
        );
    }

    #[test]
    fn other_date_forms_are_left_to_git() {
        for odd in [
            "yesterday",
            "2001-02-03",
            "2001-02-03T04:05:06",
            "2001-13-03T04:05:06 +0000",
            "2001-02-30T04:05:06 +0000",
            "Sat, 03 Feb 2001 04:05:06 +0530",
            "2001-02-03 04:05:06 +0530",
            "1015243567",
            "",
        ] {
            assert_eq!(parse_date(OsStr::new(odd)), None, "{odd:?}");
        }
    }
}
