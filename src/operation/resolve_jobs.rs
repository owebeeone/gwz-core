



/// Default global ceiling on concurrent member network operations (`--jobs`).
pub const DEFAULT_JOBS: usize = 50;
/// Global ceiling on concurrent member operations: the driver's `--jobs` value
/// when valid, otherwise [`DEFAULT_JOBS`].
pub fn resolve_jobs(requested: Option<i64>) -> usize {
    match requested {
        Some(jobs) if jobs >= 1 => jobs as usize,
        _ => DEFAULT_JOBS,
    }
}

