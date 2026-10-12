//! The transport setting's gwz-core unit rows (`dev-docs/GwzTransportOffSwitchDesign.md`
//! §9, "gwz-core, unit").
//!
//! Each test builds its own environment snapshot and writes its own files
//! under a `TempDir`, so no test reads another's files, and none sets the
//! process environment. The rows on printed commands run them in each of the
//! platform's shells (`sh`; cmd.exe and PowerShell) and git exactly as §10 prints them.

mod files;
mod fixture;
mod precedence;
mod scan;
// The words and the printed commands of design section 10 have a form for each platform (decision D11): POSIX shell,
// run through `sh`, and double-quoted paths on Windows, run through cmd.exe and PowerShell.
mod text;
