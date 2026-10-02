//! Native Git parses each bounded source; core walks only unconditional includes.
use super::*;
use std::{os::unix::ffi::OsStrExt, path::Path};

mod framing;
use framing::{Entry, Root, discovery, entries, parameters};

pub(super) const SOURCE_LIMIT: usize = 1024 * 1024;
pub(super) const PREPARATION_LIMIT: usize = 4 * 1024 * 1024;
const DISCOVERY: &[&str] = &[
    "config",
    "--no-includes",
    "--null",
    "--show-origin",
    "--show-scope",
    "--list",
];

struct Bounds {
    bytes: usize,
    entries: usize,
    visits: usize,
}
impl Bounds {
    fn charge(&mut self, bytes: usize) -> Result<(), AuthError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(AuthError::ConfigurationRefused)?;
        if self.bytes > PREPARATION_LIMIT {
            return Err(AuthError::ConfigurationRefused);
        }
        Ok(())
    }
    fn parse(&mut self, bytes: &[u8]) -> Result<Vec<Entry>, AuthError> {
        self.charge(bytes.len())?;
        let entries = entries(bytes)?;
        self.entries += entries.len();
        if self.entries > 4096 {
            return Err(AuthError::ConfigurationRefused);
        }
        Ok(entries)
    }
}

pub(super) async fn prepare(runner: &runner::Runner<'_>) -> Result<SecretBuffer, AuthError> {
    let raw = runner
        .run(DISCOVERY, &[], None, PREPARATION_LIMIT, true)
        .await?;
    let roots = discovery(&raw.0)?;
    let mut bounds = Bounds {
        bytes: 0,
        entries: 0,
        visits: 0,
    };
    bounds.charge(raw.0.len())?;
    drop(raw);
    let mut flattened = Vec::new();
    for root in roots {
        match root {
            Root::File(path) => walk(runner, path, 0, &mut bounds, &mut flattened).await?,
            Root::Command(entry) => {
                bounds.entries += 1;
                if bounds.entries > 4096 {
                    return Err(AuthError::ConfigurationRefused);
                }
                accept(runner, entry, b"/", 0, &mut bounds, &mut flattened).await?;
            }
        }
    }
    let parameters = parameters(&flattened)?;
    bounds.charge(parameters.0.len())?;
    let verification = runner
        .run(DISCOVERY, &[], Some(&parameters.0), PREPARATION_LIMIT, true)
        .await?;
    bounds.charge(verification.0.len())?;
    let verified = discovery(&verification.0)?;
    let mut actual = Vec::new();
    for root in verified {
        match root {
            Root::Command(entry) => actual.push(entry),
            Root::File(_) => return Err(AuthError::ConfigurationRefused),
        }
    }
    if actual != flattened {
        return Err(AuthError::ConfigurationRefused);
    }
    Ok(parameters)
}

fn walk<'a>(
    runner: &'a runner::Runner<'a>,
    path: SecretBuffer,
    depth: usize,
    bounds: &'a mut Bounds,
    output: &'a mut Vec<Entry>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AuthError>> + Send + 'a>> {
    Box::pin(async move {
        runner.check()?;
        bounds.visits += 1;
        if depth > 10 || bounds.visits > 128 {
            return Err(AuthError::ConfigurationRefused);
        }
        let anchor = Path::new(OsStr::from_bytes(&path.0))
            .parent()
            .ok_or(AuthError::ConfigurationRefused)?;
        let anchor = SecretBuffer(anchor.as_os_str().as_bytes().to_vec());
        let source = super::file_worker::read(runner, path).await?;
        bounds.charge(source.0.len())?;
        if source.0.is_empty() {
            return Ok(());
        }
        let parsed = runner
            .run(
                &["config", "--no-includes", "--null", "--file", "-", "--list"],
                &source.0,
                Some(&[]),
                PREPARATION_LIMIT,
                true,
            )
            .await?;
        let parsed = bounds.parse(&parsed.0)?;
        for entry in parsed {
            accept(runner, entry, &anchor.0, depth, bounds, output).await?;
        }
        Ok(())
    })
}

fn accept<'a>(
    runner: &'a runner::Runner<'a>,
    entry: Entry,
    anchor: &'a [u8],
    depth: usize,
    bounds: &'a mut Bounds,
    output: &'a mut Vec<Entry>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AuthError>> + Send + 'a>> {
    Box::pin(async move {
        if entry.name.0 == b"include.path" {
            let value = entry
                .value
                .as_ref()
                .ok_or(AuthError::ConfigurationRefused)?;
            let path = include_path(runner.config, anchor, &value.0)?;
            walk(runner, path, depth + 1, bounds, output).await?;
        } else if entry.name.0.starts_with(b"includeif.") && entry.name.0.ends_with(b".path") {
            // Never evaluate any conditional include, including future forms.
        } else {
            output.push(entry);
        }
        Ok(())
    })
}

fn include_path(config: &Config, anchor: &[u8], value: &[u8]) -> Result<SecretBuffer, AuthError> {
    if value.contains(&0) {
        return Err(AuthError::ConfigurationRefused);
    }
    let (base, value) = if let Some(rest) = value.strip_prefix(b"~/") {
        let home = config
            .environment
            .iter()
            .find(|(key, _)| key == "HOME")
            .map(|(_, value)| value.as_os_str().as_bytes())
            .ok_or(AuthError::ConfigurationRefused)?;
        if home.is_empty() || !Path::new(OsStr::from_bytes(home)).is_absolute() {
            return Err(AuthError::ConfigurationRefused);
        }
        (home, rest)
    } else {
        if value.starts_with(b"~") || value.starts_with(b"%(prefix)") {
            return Err(AuthError::ConfigurationRefused);
        }
        (anchor, value)
    };
    framing::absolute(base, value)
}

cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }
