//! The carrier's rows (plan 2.1). Fixture tests: nothing here is compiled into a build that is not a test build.
mod address_rows;
mod address_walk_rows;
mod class_rows;
mod ending_rows;
mod handshake_body_rows;
mod handshake_client_rows;
mod handshake_local_rows;
mod handshake_rows;
mod handshake_support;
mod local_rows;
mod pre_open_rows;
mod pressure_rows;
mod sandbox_rows;
mod seam_rows;
mod server_files_fork_rows;
mod server_files_rows;
mod stream_rows;
mod support;
mod wire_rows;
cfg_if::cfg_if! { if #[cfg(unix)] { mod directory_rows; mod walk_entry_rows; } else if #[cfg(windows)] { mod windows_rows; } }
cfg_if::cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android"))] {
        mod hold_rows;
    } else if #[cfg(target_vendor = "apple")] {
        mod probe_rows;
    }
}
