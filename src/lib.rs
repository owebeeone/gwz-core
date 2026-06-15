pub mod runtime;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_package_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
