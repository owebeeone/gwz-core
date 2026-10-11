//! The transport setting's gwz-core unit rows (`dev-docs/GwzTransportOffSwitchDesign.md`
//! §9, "gwz-core, unit").
//!
//! Each test builds its own environment snapshot and writes its own files
//! under a `TempDir`, so no test reads another's files, and none sets the
//! process environment. The rows on printed commands run them through `sh`
//! and git exactly as §10 prints them.

mod files;
mod fixture;
mod precedence;
mod scan;
// The words and the printed commands of design section 10 are POSIX shell, and their rows quote POSIX paths and run
// the commands through `sh`. What a Windows shell is told to run is not designed yet (plan step 3.1's report), so these
// rows wait for it.
cfg_if::cfg_if! { if #[cfg(unix)] { mod text; } }
