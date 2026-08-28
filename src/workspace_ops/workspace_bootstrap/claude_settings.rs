//! The checked-in Claude Code deny rule that keeps agents out of `gwz.conf/`.
//!
//! Emitted and refreshed by the same bootstrap that emits `AGENTS_GWZ.md`, so a workspace
//! that gets the prose also gets the enforcement. Unlike `AGENTS_GWZ.md` this file is not
//! gwz's to own — a workspace may already have Claude Code settings for entirely unrelated
//! reasons — so the update is a *merge*: add the deny entries that are missing, touch
//! nothing else, and back off entirely from a file we cannot parse.
//!
//! JSON is read and written here without a JSON dependency: JSON is a subset of YAML 1.2,
//! so the crate's existing `serde_yaml` parses it, and the emitter below is the only new
//! code. Adding `serde_json` for one three-line config file is not warranted.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde_yaml::Value;

use crate::model::{ErrorCode, ModelError, ModelResult};

pub(crate) const CLAUDE_SETTINGS_PATH: &str = ".claude/settings.json";

/// The deny rules gwz maintains.
///
/// Syntax verified against the live Claude Code docs, not recalled:
///
/// * `https://code.claude.com/docs/en/settings`, "Edit a settings file" — permission rules
///   live under a top-level `permissions` object holding `allow` / `deny` (/ `ask`) arrays,
///   and settings files are strict JSON (a comment or trailing comma is a syntax error).
/// * `https://code.claude.com/docs/en/permissions`, "Permission rule syntax" — rules take
///   the form `Tool` or `Tool(specifier)`.
/// * Same page, "Read and Edit" — "Claude Code checks file permissions against
///   `Edit(path)` and `Read(path)` rules only"; a path rule written for `Write`,
///   `MultiEdit` or `NotebookEdit` is accepted but never consulted, and warns at startup.
///   `Edit(...)` is therefore the rule that covers every file-mutating tool, and a
///   `Read(...)` deny is deliberately NOT used — agents must still be able to read the
///   workspace state they are forbidden to write.
/// * Same section, pattern table — `/path` is "Path relative to the settings source", and
///   for project settings at `.claude/settings.json` that resolves to
///   `<primary working directory>/path`. The comparison table lists `Edit(/src/**)` as
///   matching "in any rule type" and only at its anchored location, so the anchored form
///   is supported for deny rules and says exactly what is meant: the `gwz.conf` at the
///   root of the project this settings file belongs to.
///
/// **What layer 3 does not cover.** An earlier revision used the bare `gwz.conf/**` form
/// and justified it by sessions started in a member subdirectory. That reasoning was
/// wrong twice over. As a deny rule the bare form matches a `gwz.conf` at any depth
/// *under* the current directory but explicitly not in a *parent*, so it does not reach
/// the workspace's `gwz.conf` from `<workspace>/repos/app` either — and more decisively, a
/// session started inside a member repo loads that member's own project settings, not the
/// workspace's, so this file is not even read there. Layer 3 therefore protects
/// workspace-root sessions only. Layer 2 — the conf-integrity gate — is the
/// cwd-independent defence, and the one that catches an edit however it was made.
pub(crate) const CONF_DENY_RULES: [&str; 1] = ["Edit(/gwz.conf/**)"];

const PERMISSIONS_KEY: &str = "permissions";
const DENY_KEY: &str = "deny";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClaudeSettingsUpdate {
    Created,
    Updated,
    Unchanged,
    /// The existing file was left exactly as it was; `reason` is for the caller's output.
    Skipped(String),
}

impl ClaudeSettingsUpdate {
    pub(crate) fn changed(&self) -> bool {
        matches!(self, Self::Created | Self::Updated)
    }

    pub(crate) fn warning(&self) -> Option<String> {
        match self {
            Self::Skipped(reason) => Some(format!(
                "left {CLAUDE_SETTINGS_PATH} untouched ({reason}); \
                 add {} to its permissions.deny by hand",
                CONF_DENY_RULES.join(", ")
            )),
            Self::Created | Self::Updated | Self::Unchanged => None,
        }
    }
}

/// Create or idempotently merge the `gwz.conf` deny rules into `.claude/settings.json`.
pub(crate) fn ensure_claude_settings(
    root: &Path,
    dry_run: bool,
) -> ModelResult<ClaudeSettingsUpdate> {
    let path = root.join(CLAUDE_SETTINGS_PATH);
    let existing = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(ModelError::new(ErrorCode::IoError, error.to_string())),
    };

    let (update, contents) = match existing {
        None => (ClaudeSettingsUpdate::Created, Some(fresh_settings())),
        Some(text) => match merge_deny_rules(&text) {
            Err(reason) => (ClaudeSettingsUpdate::Skipped(reason), None),
            Ok(None) => (ClaudeSettingsUpdate::Unchanged, None),
            Ok(Some(merged)) => (ClaudeSettingsUpdate::Updated, Some(merged)),
        },
    };

    if let Some(contents) = contents
        && !dry_run
    {
        // `write_atomic` creates `.claude/` on the way through and publishes durably.
        crate::artifact::write_atomic(&path, contents)?;
    }
    Ok(update)
}

/// The file gwz writes when there is none: exactly the deny entries, nothing else.
fn fresh_settings() -> String {
    let mut out = String::from("{\n  \"permissions\": {\n    \"deny\": [\n");
    for (index, rule) in CONF_DENY_RULES.iter().enumerate() {
        out.push_str("      ");
        push_json_string(rule, &mut out);
        if index + 1 < CONF_DENY_RULES.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("    ]\n  }\n}\n");
    out
}

/// `Ok(None)` when every rule is already present, `Ok(Some(text))` for the merged file,
/// `Err(reason)` when the file must be left alone.
fn merge_deny_rules(text: &str) -> Result<Option<String>, String> {
    // JSON is a YAML 1.2 subset, so the crate's existing YAML parser reads it. Anything it
    // rejects — a duplicate key, a truncated file, a non-string object key — is the
    // "unparseable" case we must not touch. Note the asymmetry: YAML is the LOOSER
    // grammar, so some files Claude Code would reject as invalid JSON (a trailing comma, a
    // leading `#` comment, anchors and aliases, a `---` separator) parse here and are
    // re-emitted as valid JSON. That normalisation is verified by the round-trip check
    // below before it is written, but it is a rewrite, not a skip.
    let mut root: Value = serde_yaml::from_str(text).map_err(|err| err.to_string())?;
    let object = root
        .as_mapping_mut()
        .ok_or_else(|| "top level is not a JSON object".to_owned())?;

    let permissions_key = Value::String(PERMISSIONS_KEY.to_owned());
    if !object.contains_key(&permissions_key) {
        object.insert(permissions_key.clone(), Value::Mapping(Default::default()));
    }
    let permissions = object
        .get_mut(&permissions_key)
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| format!("`{PERMISSIONS_KEY}` is not a JSON object"))?;

    let deny_key = Value::String(DENY_KEY.to_owned());
    if !permissions.contains_key(&deny_key) {
        permissions.insert(deny_key.clone(), Value::Sequence(Vec::new()));
    }
    let deny = permissions
        .get_mut(&deny_key)
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("`{PERMISSIONS_KEY}.{DENY_KEY}` is not a JSON array"))?;

    let mut added = false;
    for rule in CONF_DENY_RULES {
        if !deny.iter().any(|entry| entry.as_str() == Some(rule)) {
            // Append: existing entries keep their order and their meaning.
            deny.push(Value::String(rule.to_owned()));
            added = true;
        }
    }
    if !added {
        return Ok(None);
    }

    let mut out = String::new();
    push_json(&root, 0, &mut out).ok_or_else(|| "file holds non-JSON values".to_owned())?;
    out.push('\n');

    // Verify the transform instead of trusting it. `push_json` re-renders a document that
    // arrived through a YAML parser, and that path is lossy in ways JSON cares about: a
    // number too large for `u64` re-emits as digits the reader then refuses, a float can
    // narrow to an integer. Re-reading what we are about to write and demanding it equal
    // the value we intended turns those into a `Skipped` — file untouched, warning
    // raised — rather than a rewrite that quietly changes meaning or, worse, one gwz can
    // never read again. One residual this check cannot see (verification NF-2): an
    // unresolvable exponent past f64 range (`1e400`) is already a STRING by the time the
    // parser hands it to us, so the round trip compares equal and the retyped scalar is
    // written; the lossy-shapes test pins that retype so it cannot drift unnoticed.
    let verified: Value =
        serde_yaml::from_str(&out).map_err(|err| format!("emitted JSON did not re-read: {err}"))?;
    if verified != root {
        return Err("emitted JSON did not round-trip to the same value".to_owned());
    }
    Ok(Some(out))
}

/// Emit `value` as JSON. `None` means the document holds something JSON cannot express
/// (a non-string object key, a YAML tag, a non-finite number), in which case the caller
/// leaves the file alone rather than mangling it.
fn push_json(value: &Value, indent: usize, out: &mut String) -> Option<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                let _ = write!(out, "{value}");
            } else if let Some(value) = number.as_u64() {
                let _ = write!(out, "{value}");
            } else {
                let value = number.as_f64()?;
                if !value.is_finite() {
                    return None;
                }
                let _ = write!(out, "{value}");
            }
        }
        Value::String(value) => push_json_string(value, out),
        Value::Sequence(items) => {
            if items.is_empty() {
                out.push_str("[]");
            } else {
                out.push_str("[\n");
                for (index, item) in items.iter().enumerate() {
                    push_indent(indent + 1, out);
                    push_json(item, indent + 1, out)?;
                    out.push_str(if index + 1 < items.len() { ",\n" } else { "\n" });
                }
                push_indent(indent, out);
                out.push(']');
            }
        }
        Value::Mapping(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
            } else {
                out.push_str("{\n");
                let count = entries.len();
                for (index, (key, item)) in entries.iter().enumerate() {
                    push_indent(indent + 1, out);
                    push_json_string(key.as_str()?, out);
                    out.push_str(": ");
                    push_json(item, indent + 1, out)?;
                    out.push_str(if index + 1 < count { ",\n" } else { "\n" });
                }
                push_indent(indent, out);
                out.push('}');
            }
        }
        Value::Tagged(_) => return None,
    }
    Some(())
}

fn push_indent(indent: usize, out: &mut String) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

fn push_json_string(value: &str, out: &mut String) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::tests::TempDir;

    fn settings(temp: &TempDir) -> String {
        fs::read_to_string(temp.path().join(CLAUDE_SETTINGS_PATH)).unwrap()
    }

    #[test]
    fn a_workspace_without_settings_gets_exactly_the_deny_entries() {
        let temp = TempDir::new("claude-settings-create");

        assert_eq!(
            ensure_claude_settings(temp.path(), false).unwrap(),
            ClaudeSettingsUpdate::Created
        );

        assert_eq!(
            settings(&temp),
            "{\n  \"permissions\": {\n    \"deny\": [\n      \"Edit(/gwz.conf/**)\"\n    ]\n  }\n}\n"
        );
    }

    #[test]
    fn re_running_is_a_noop_and_rewrites_nothing() {
        let temp = TempDir::new("claude-settings-idempotent");
        ensure_claude_settings(temp.path(), false).unwrap();
        let first = settings(&temp);

        assert_eq!(
            ensure_claude_settings(temp.path(), false).unwrap(),
            ClaudeSettingsUpdate::Unchanged
        );

        assert_eq!(settings(&temp), first);
    }

    #[test]
    fn merging_preserves_every_unrelated_key_and_appends_the_missing_rule() {
        let temp = TempDir::new("claude-settings-merge");
        fs::create_dir_all(temp.path().join(".claude")).unwrap();
        fs::write(
            temp.path().join(CLAUDE_SETTINGS_PATH),
            "{\n\t\"model\": \"claude-opus-4-8\",\n\t\"permissions\": {\n\t\t\"allow\": [\"Bash(cargo test)\"],\n\t\t\"deny\": [\"Read(./.env)\"]\n\t},\n\t\"env\": {\"DEBUG\": \"1\"}\n}\n",
        )
        .unwrap();

        assert_eq!(
            ensure_claude_settings(temp.path(), false).unwrap(),
            ClaudeSettingsUpdate::Updated
        );

        // Tab-indented JSON parses, every unrelated key survives, and the rule is
        // appended after the entries that were already there.
        let merged = settings(&temp);
        assert_eq!(
            merged,
            "{\n  \"model\": \"claude-opus-4-8\",\n  \"permissions\": {\n    \"allow\": [\n      \"Bash(cargo test)\"\n    ],\n    \"deny\": [\n      \"Read(./.env)\",\n      \"Edit(/gwz.conf/**)\"\n    ]\n  },\n  \"env\": {\n    \"DEBUG\": \"1\"\n  }\n}\n"
        );
        assert_eq!(
            ensure_claude_settings(temp.path(), false).unwrap(),
            ClaudeSettingsUpdate::Unchanged
        );
    }

    #[test]
    fn a_settings_file_without_a_permissions_block_gains_one() {
        let temp = TempDir::new("claude-settings-no-permissions");
        fs::create_dir_all(temp.path().join(".claude")).unwrap();
        fs::write(
            temp.path().join(CLAUDE_SETTINGS_PATH),
            "{\"$schema\": \"https://json.schemastore.org/claude-code-settings.json\"}",
        )
        .unwrap();

        ensure_claude_settings(temp.path(), false).unwrap();

        assert_eq!(
            settings(&temp),
            "{\n  \"$schema\": \"https://json.schemastore.org/claude-code-settings.json\",\n  \"permissions\": {\n    \"deny\": [\n      \"Edit(/gwz.conf/**)\"\n    ]\n  }\n}\n"
        );
    }

    #[test]
    fn an_unparseable_settings_file_is_left_untouched_with_a_warning() {
        let temp = TempDir::new("claude-settings-unparseable");
        fs::create_dir_all(temp.path().join(".claude")).unwrap();
        let broken = "{\"permissions\": {\"deny\": [\n";
        fs::write(temp.path().join(CLAUDE_SETTINGS_PATH), broken).unwrap();

        let update = ensure_claude_settings(temp.path(), false).unwrap();

        assert!(matches!(update, ClaudeSettingsUpdate::Skipped(_)));
        assert!(!update.changed());
        assert!(update.warning().unwrap().contains(CLAUDE_SETTINGS_PATH));
        assert_eq!(settings(&temp), broken);
    }

    #[test]
    fn a_permissions_block_of_the_wrong_shape_is_left_untouched() {
        for body in [
            "[\"not an object\"]",
            "{\"permissions\": \"strict\"}",
            "{\"permissions\": {\"deny\": \"Edit(/gwz.conf/**)\"}}",
        ] {
            let temp = TempDir::new("claude-settings-shape");
            fs::create_dir_all(temp.path().join(".claude")).unwrap();
            fs::write(temp.path().join(CLAUDE_SETTINGS_PATH), body).unwrap();

            let update = ensure_claude_settings(temp.path(), false).unwrap();

            assert!(
                matches!(update, ClaudeSettingsUpdate::Skipped(_)),
                "expected {body} to be skipped, got {update:?}"
            );
            assert_eq!(settings(&temp), body);
        }
    }

    #[test]
    fn a_dry_run_writes_nothing_but_still_reports_the_change() {
        let temp = TempDir::new("claude-settings-dry-run");

        assert_eq!(
            ensure_claude_settings(temp.path(), true).unwrap(),
            ClaudeSettingsUpdate::Created
        );

        assert!(!temp.path().join(CLAUDE_SETTINGS_PATH).exists());
    }

    #[test]
    fn every_lossy_json_shape_ends_skipped_or_verified_never_corrupted() {
        // [P2-3] The review's fixture cluster. The invariant is absolute: gwz either
        // leaves the file byte-identical, or writes something that re-reads to exactly the
        // value it meant — never a file whose meaning drifted, and never one gwz's own
        // reader will later reject.
        let cases = [
            ("1e22 self-unreadable", "{\"n\": 1e22}"),
            ("1e400 unresolvable exponent", "{\"n\": 1e400}"),
            ("float that narrows", "{\"n\": 1.0}"),
            ("trailing comma", "{\"a\": [1,]}"),
            ("leading comment", "# local\n{\"a\": 1}"),
            ("anchors and aliases", "{\"a\": &x 1, \"b\": *x}"),
        ];

        for (name, source) in cases {
            let temp = TempDir::new("claude-settings-lossy");
            fs::create_dir_all(temp.path().join(".claude")).unwrap();
            fs::write(temp.path().join(CLAUDE_SETTINGS_PATH), source).unwrap();

            let update = ensure_claude_settings(temp.path(), false).unwrap();
            let after = settings(&temp);

            match update {
                ClaudeSettingsUpdate::Skipped(_) => {
                    assert_eq!(after, source, "{name}: skipped but the file changed");
                }
                _ => {
                    // Written: it must re-read, and it must carry the rule.
                    let reparsed: Value = serde_yaml::from_str(&after).unwrap_or_else(|err| {
                        panic!("{name}: gwz wrote what it cannot read: {err}")
                    });
                    let deny = reparsed
                        .get("permissions")
                        .and_then(|permissions| permissions.get("deny"))
                        .and_then(Value::as_sequence)
                        .unwrap_or_else(|| panic!("{name}: no deny array"));
                    assert!(
                        deny.iter()
                            .any(|entry| entry.as_str() == Some(CONF_DENY_RULES[0])),
                        "{name}: rule missing"
                    );
                    // The pinned NF-2 residual: the parser resolved the overflowing
                    // literal to a string before the round-trip check could see a
                    // number, so the retype is written. Pin it so it cannot drift
                    // unnoticed — and so this test claims no more than it proves.
                    if name.starts_with("1e400") {
                        assert_eq!(
                            reparsed.get("n"),
                            Some(&Value::String("1e400".into())),
                            "{name}: the pinned retype moved"
                        );
                    }
                    // And a second pass must be a clean no-op, never an endless rewrite.
                    assert_eq!(
                        ensure_claude_settings(temp.path(), false).unwrap(),
                        ClaudeSettingsUpdate::Unchanged,
                        "{name}: not idempotent"
                    );
                }
            }
        }
    }

    #[test]
    fn a_number_gwz_cannot_re_read_is_skipped_rather_than_written() {
        // The sharpest case: 1e22 re-emits as 23 digits that serde_yaml then refuses, so
        // without the round-trip check gwz would write a file it could never read again
        // and would classify its own output as unparseable forever after.
        let temp = TempDir::new("claude-settings-unreadable-number");
        fs::create_dir_all(temp.path().join(".claude")).unwrap();
        let source = "{\"n\": 1e22}";
        fs::write(temp.path().join(CLAUDE_SETTINGS_PATH), source).unwrap();

        let update = ensure_claude_settings(temp.path(), false).unwrap();

        assert!(
            matches!(update, ClaudeSettingsUpdate::Skipped(_)),
            "expected a skip, got {update:?}"
        );
        assert_eq!(settings(&temp), source);
        assert!(update.warning().unwrap().contains(CLAUDE_SETTINGS_PATH));
    }

    #[test]
    fn json_round_trips_through_the_yaml_reader_and_the_json_emitter() {
        // The load-bearing assumption of this module: everything JSON can express
        // survives parse + emit unchanged in meaning.
        let source = "{\"n\": null, \"t\": true, \"i\": -12, \"f\": 1.5, \"s\": \"a\\\"b\\\\c\\nd\\u0007\", \"a\": [], \"o\": {}, \"deep\": [{\"k\": [1, 2]}]}";
        let value: Value = serde_yaml::from_str(source).unwrap();
        let mut emitted = String::new();
        push_json(&value, 0, &mut emitted).unwrap();

        let reparsed: Value = serde_yaml::from_str(&emitted).unwrap();
        assert_eq!(reparsed, value);
        assert!(
            emitted.contains(r#""s": "a\"b\\c\nd\u0007""#),
            "escaping drifted: {emitted}"
        );
    }
}
