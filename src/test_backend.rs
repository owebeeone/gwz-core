//! One immutable process-wide selection shared by both test factories.
use std::sync::OnceLock;
pub(crate) struct Modes {
    pub(crate) fake_git: bool,
    pub(crate) fake_filesystem: bool,
}
pub(crate) fn modes() -> &'static Modes {
    static MODES: OnceLock<Modes> = OnceLock::new();
    MODES.get_or_init(|| {
        fn selected(name: &str, default: bool) -> bool {
            match std::env::var(name) {
                Ok(value) if value == "fake" => true,
                Ok(value) if value == "real" => false,
                Err(std::env::VarError::NotPresent) => default,
                _ => panic!("{name} must be real or fake"),
            }
        }
        let modes = Modes {
            fake_git: selected("GWZ_TEST_GIT", true),
            fake_filesystem: selected("GWZ_TEST_FS", false),
        };
        assert!(
            !modes.fake_filesystem || modes.fake_git,
            "native Git cannot use a fake filesystem"
        );
        modes
    })
}
