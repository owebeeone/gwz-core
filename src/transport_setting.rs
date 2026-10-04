//! The transport setting (`dev-docs/GwzTransportOffSwitchDesign.md`, TR1.5,
//! the transport release plan's off switch): which transport carries a
//! network operation, gwz's own or libgit2's native one, and the repository
//! values that do not select it. Both drivers call it beside
//! `transport_scope`: gwz-cli for a command whose request is in transport
//! scope, and gwz-py at each network operation's native entry (design §2,
//! §7).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | The flag, then `GWZ_TRANSPORT`, then `gwz.transport` in the user's global git configuration: the first form that carries a value decides, the forms below it are neither read nor checked, and with none `gwz` applies | §2 "Precedence" |
//! | Each form takes `gwz` or `native`. The variable ignores ASCII case and surrounding white space and is unset when empty; the key's value ignores case; any other value, an empty value, a key with no value and a variable that is not UTF-8 are refused. Within a file the last value wins | §2 "Values" |
//! | The global files are git's `--global` set, located from the snapshot's `XDG_CONFIG_HOME` and `HOME`, `$HOME/.gitconfig`'s value winning; an empty or relative `HOME`, or a relative `XDG_CONFIG_HOME`, names no file | §2 "The files", D5 |
//! | Each file is opened without a repository: `include.path` is followed, a `~/` one against libgit2's process-wide home, and no `includeIf` applies; a value reached through an include is reported as included | §2 "Includes", E2 |
//! | A global file that cannot be read carries no value and is reported as skipped; one that cannot be parsed is refused with libgit2's message | §2, D4 |
//! | A repository's own value never selects the transport: the scan lists it for the drivers' notes, from the request's targets only, in regular files of at most 1 MiB found without opening the repository, and never refuses or blocks | §3, D6, E9 |
//! | Every value, path and identifier rendered follows E1, and the refusals and the pieces of the notes take §10's words | §10, E1 |
//!
//! The module keeps no state and reads no process environment: the driver
//! passes the flag's value and its environment snapshot. libgit2 reads its
//! own process-wide home, fixed when it initialises, for a `~/` include
//! (design §2). Like the transport's other sites, the module is candidate
//! code until 1.1.0 S7.1, on Linux and macOS until 1.1.0 S4.5 opens the
//! transport's Windows sites (design §7).

mod global;
mod scan;
mod text;

use std::path::{Path, PathBuf};

use crate::RequestMeta;
use crate::session_host::EnvironmentSnapshot;
use crate::transport_scope::Operation;

pub use text::path_text;

/// The variable form's name (design §1).
const VARIABLE: &str = "GWZ_TRANSPORT";

/// The global configuration form's key (design §1).
const KEY: &str = "gwz.transport";

/// The transport that carries a network operation (design §1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transport {
    /// gwz's own SSH and HTTPS transport, the default.
    Gwz,
    /// libgit2's native transport, as gwz 1.0 used.
    Native,
}

impl Transport {
    /// The word every form takes and every report uses, `gwz` or `native`
    /// (design §10, D2).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gwz => "gwz",
            Self::Native => "native",
        }
    }

    /// The transport that `value` names, ignoring ASCII case, as the variable
    /// and the key read it (design §2). Nothing else names one, git's other
    /// boolean spellings included.
    fn named(value: &[u8]) -> Option<Self> {
        [Self::Gwz, Self::Native]
            .into_iter()
            .find(|transport| value.eq_ignore_ascii_case(transport.name().as_bytes()))
    }
}

/// The form that decided (design §5, and `source` in §10's JSON).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Source {
    /// gwz-cli's `--transport`.
    Flag,
    /// `GWZ_TRANSPORT`.
    Environment,
    /// `gwz.transport` in a global file, at this location.
    GlobalConfiguration(Location),
    /// No form carries a value, so `gwz` applies.
    Default,
}

impl Source {
    /// The JSON's word for the source (design §10).
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Environment => "environment",
            Self::GlobalConfiguration(_) => "global_configuration",
            Self::Default => "default",
        }
    }
}

/// Where a value of `gwz.transport` is (design §10, E2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Location {
    /// The file gwz opened: a global file, or a repository's `config` or
    /// `config.worktree`.
    pub file: PathBuf,
    /// Whether the value came through `include.path`, from a file that `file`
    /// includes. git2 exposes the entry's include depth but not the included
    /// file's path, so the messages give a command that locates it.
    pub included: bool,
}

/// The resolved setting (design §2, §5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Setting {
    /// The transport selected.
    pub transport: Transport,
    /// The form that selected it.
    pub source: Source,
    /// The global files read, in the order read, the XDG file first. Empty
    /// unless the global configuration was consulted.
    pub read: Vec<PathBuf>,
    /// The global files that exist but could not be read, in the same order.
    /// They carry no value (design §2, D4).
    pub skipped: Vec<PathBuf>,
}

/// A malformed form that would decide, refused before any effect (design
/// §2, §10). `message` gives its words.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// `GWZ_TRANSPORT` names neither transport. `value` is the variable
    /// without its surrounding white space.
    Environment { value: String },
    /// `GWZ_TRANSPORT` is not valid UTF-8.
    EnvironmentNotUtf8,
    /// The deciding `gwz.transport` names neither transport. `value` is the
    /// value as written, invalid UTF-8 replaced, or `None` for a key with no
    /// value.
    Configuration {
        location: Location,
        value: Option<String>,
    },
    /// A global file, or a file it includes, could not be read as git
    /// configuration. `cause` is libgit2's message, which names the file and
    /// the line.
    Unparsable { file: PathBuf, cause: String },
}

/// Whose words a message takes (design §10): gwz-cli's, which has the
/// `--transport` flag, or gwz-py's, which in 1.1.0 has no flag form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Driver {
    Cli,
    Python,
}

/// The transport setting, from the flag's value, which only gwz-cli has, and
/// the environment snapshot the driver captured (design §2, §7).
///
/// The first form that carries a value decides, and the forms below it are
/// neither read nor checked: a given flag leaves the snapshot unread, and a
/// decisive variable leaves the global files unread. The global files are
/// read from the snapshot's `HOME` and `XDG_CONFIG_HOME`, never from the
/// process environment or the passwd entry.
pub fn resolve(
    flag: Option<Transport>,
    environment: &EnvironmentSnapshot,
) -> Result<Setting, Refusal> {
    let decided = |transport, source| Setting {
        transport,
        source,
        read: Vec::new(),
        skipped: Vec::new(),
    };
    if let Some(transport) = flag {
        return Ok(decided(transport, Source::Flag));
    }
    if let Some(transport) = variable(environment)? {
        return Ok(decided(transport, Source::Environment));
    }
    global::resolve(environment)
}

/// The variable's transport, or `None` when it is unset or empty (design
/// §2).
fn variable(environment: &EnvironmentSnapshot) -> Result<Option<Transport>, Refusal> {
    let Some(value) = environment.get(VARIABLE) else {
        return Ok(None);
    };
    let Some(value) = value.as_os_str().to_str() else {
        return Err(Refusal::EnvironmentNotUtf8);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    match Transport::named(value.as_bytes()) {
        Some(transport) => Ok(Some(transport)),
        None => Err(Refusal::Environment {
            value: value.to_owned(),
        }),
    }
}

/// The repository whose own configuration holds an ignored value (design §3,
/// and `<scope>` in §10).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Scope {
    /// The workspace root's repository.
    Root,
    /// A member's repository, by its member ID.
    Member(String),
}

impl Scope {
    /// The JSON's word for the scope, `root` or `member` (design §10).
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Member(_) => "member",
        }
    }

    /// The member's ID, or `None` for the root (the JSON's `member_id`).
    pub fn member_id(&self) -> Option<&str> {
        match self {
            Self::Root => None,
            Self::Member(id) => Some(id),
        }
    }
}

/// A `gwz.transport` in a repository's own configuration, which never selects
/// the transport and is never checked (design §3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IgnoredValue {
    /// The repository whose file holds it.
    pub scope: Scope,
    /// The file, and whether the value came through its `include.path`.
    pub location: Location,
    /// The value as written, invalid UTF-8 replaced, or `None` for a key with
    /// no value. As git reads one key, it is the last value the file reaches.
    pub value: Option<String>,
}

/// Each `gwz.transport` in the own configuration of the repositories that a
/// request of `operation` targets, resolved as the operation resolves them
/// from `start` and `meta`: the `.git/config` and `config.worktree` of the
/// workspace root and of each member it targets (design §3).
///
/// One entry per file, whichever transport is selected. The scan never
/// refuses or blocks: a workspace, a repository or a file it cannot read gets
/// no entry, and the operation reports its own errors.
pub fn ignored_values(operation: Operation, start: &Path, meta: &RequestMeta) -> Vec<IgnoredValue> {
    scan::ignored_values(operation, start, meta)
}

/// The last `gwz.transport` that a file reaches, as git reads one key.
struct Found {
    /// Whether it came through `include.path`: libgit2's entry has an include
    /// depth above 0 (design §2).
    included: bool,
    /// The value's bytes as written, or `None` for a key with no value.
    value: Option<Vec<u8>>,
}

/// The last `gwz.transport` that `file` reaches. libgit2 opens the file
/// without a repository, so `include.path` is followed, a `~/` path against
/// its process-wide home, and no `includeIf` applies (design §2, §3). Its
/// error names the file, the included one for an error in an included file,
/// and the line.
fn last_value(file: &Path) -> Result<Option<Found>, git2::Error> {
    let config = git2::Config::open(file)?;
    match config.get_entry(KEY) {
        Ok(entry) => Ok(Some(Found {
            included: entry.include_depth() > 0,
            value: entry.has_value().then(|| entry.value_bytes().to_vec()),
        })),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

cfg_if::cfg_if! {
    if #[cfg(all(test, unix))] {
        mod tests;
    }
}
