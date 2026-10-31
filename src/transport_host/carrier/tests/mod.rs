//! The carrier's rows (plan 2.1). Fixture tests: nothing here is compiled into a build that is not a test build.
mod class_rows;
mod ending_rows;
mod local_rows;
mod sandbox_rows;
mod seam_rows;
mod stream_rows;
mod support;
mod wire_rows;
cfg_if::cfg_if! { if #[cfg(unix)] { mod directory_rows; } else if #[cfg(windows)] { mod windows_rows; } }
