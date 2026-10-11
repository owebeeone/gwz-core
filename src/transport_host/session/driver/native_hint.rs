//! How the missing-git message tells the user to use libgit2's native transport instead (operator decision of
//! 2026-10-11): the flag where the caller has one, else the environment switch, which `cmd` and PowerShell set
//! differently, so a Windows hint shows both.

/// The end of the missing-git message. `cli_hint` is whether the caller has `--transport`; `windows_shells` whether
/// the switch is set in `cmd` or PowerShell.
pub(in crate::transport_host) fn native_switch_hint(
    cli_hint: bool,
    windows_shells: bool,
) -> &'static str {
    if cli_hint {
        "run with --transport native to use libgit2's native transport, as gwz 1.0 did"
    } else if windows_shells {
        "set GWZ_TRANSPORT=native (cmd) or $env:GWZ_TRANSPORT = \"native\" (PowerShell) to use libgit2's native transport, as gwz 1.0 did"
    } else {
        "set GWZ_TRANSPORT=native to use libgit2's native transport, as gwz 1.0 did"
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        use super::{SshOpenFailure, WINDOWS_SHELLS};
        use gwz_transport::protocol::{AuthMethod, Effect, ErrorCode, Facts, Failure, SetupFailureCause};

        #[test]
        fn the_missing_git_hint_shows_both_windows_shells_and_the_flag_where_there_is_one() {
            // The environment switch is `set` in `cmd` and an assignment in PowerShell, so a Windows hint shows both.
            let windows = native_switch_hint(false, true);
            assert!(windows.contains("set GWZ_TRANSPORT=native (cmd)"), "{windows}");
            assert!(windows.contains("$env:GWZ_TRANSPORT = \"native\" (PowerShell)"), "{windows}");
            assert!(windows.ends_with("to use libgit2's native transport, as gwz 1.0 did"), "{windows}");
            // Elsewhere the one form stays as it was.
            assert_eq!(
                native_switch_hint(false, false),
                "set GWZ_TRANSPORT=native to use libgit2's native transport, as gwz 1.0 did"
            );
            // A caller with a flag is pointed at the flag on every platform.
            for windows_shells in [false, true] {
                assert_eq!(
                    native_switch_hint(true, windows_shells),
                    "run with --transport native to use libgit2's native transport, as gwz 1.0 did"
                );
            }
        }

        #[test]
        fn the_missing_git_message_ends_with_this_platforms_hint() {
            let missing = Failure {
                detail: None,
                setup_cause: Some(SetupFailureCause::NotFound),
                code: ErrorCode::Unavailable,
                effect: Effect::None,
                facts: Some(Facts { method: AuthMethod::Gh, ..Facts::default() }),
            };
            let failed = SshOpenFailure(missing, None, 9_000);
            let environment = failed.model_error(false).unwrap().message;
            assert!(environment.contains("found no `git`"), "{environment}");
            assert!(
                environment.ends_with(&format!("{}.", native_switch_hint(false, WINDOWS_SHELLS))),
                "{environment}"
            );
            let flag = failed.model_error(true).unwrap().message;
            assert!(
                flag.ends_with(&format!("{}.", native_switch_hint(true, WINDOWS_SHELLS))),
                "{flag}"
            );
        }
    }
}
