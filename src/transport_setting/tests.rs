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
mod text;
