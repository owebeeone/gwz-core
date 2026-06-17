



/// Default maximum concurrent connections to any one host.
pub const DEFAULT_MAX_PER_HOST: usize = 8;

/// Maximum concurrent operations against any one host: the driver's value when
/// valid, otherwise [`DEFAULT_MAX_PER_HOST`].
pub fn resolve_per_host(requested: Option<i64>) -> usize {
    match requested {
        Some(limit) if limit >= 1 => limit as usize,
        _ => DEFAULT_MAX_PER_HOST,
    }
}

