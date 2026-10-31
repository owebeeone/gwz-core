//! R3b of the Phase 1 remediation (contract section 7; the same class as Code P2-2): the SSH password helpers of the
//! connection host's shared endpoint instances run the helper of the context of the operation each opening serves, and
//! never the runtime's or another operation's. The context reaches the opening from the adapter's connect
//! (`transport_host/connection_host/domain_ports_tests.rs`); these rows are the other link, from the opening to the
//! helper a lookup would run.
use super::*;
use crate::git::endpoint::https_auth::{Config, HelperSlots, OperationAuth};
use gwz_connection::{AuthContext, AuthScope};
use std::{ffi::OsString, path::PathBuf};

fn helper(name: &str) -> Config {
    Config {
        executable: PathBuf::from(name),
        environment: vec![(OsString::from("GH_TOKEN"), OsString::from(name))],
    }
}
fn opening_of(auth: Option<AuthContext>) -> Opening {
    Opening {
        auth,
        ..Opening::default()
    }
}
fn selected(helpers: &Helpers, opening: &Opening) -> io::Result<PathBuf> {
    helpers
        .helper_for(opening)
        .map(|config| config.executable.clone())
}

#[test]
fn each_opening_runs_the_helper_of_its_own_operations_context() {
    let helpers = Helpers::per_operation(HelperSlots::new());
    let (a, b) = (
        opening_of(Some(OperationAuth::context(helper("helper-a"), None))),
        opening_of(Some(OperationAuth::context(helper("helper-b"), None))),
    );
    // One `Helpers` serves both, in either order, and again: it keeps no helper of its own.
    assert_eq!(selected(&helpers, &a).unwrap(), PathBuf::from("helper-a"));
    assert_eq!(selected(&helpers, &b).unwrap(), PathBuf::from("helper-b"));
    assert_eq!(selected(&helpers, &a).unwrap(), PathBuf::from("helper-a"));
}

#[test]
fn an_opening_without_a_helper_context_has_no_helper() {
    let helpers = Helpers::per_operation(HelperSlots::new());
    let refused = |opening: Opening| selected(&helpers, &opening).unwrap_err().kind();
    // No context at all (the operation stopped waiting before the connect began), and a context of another provider.
    assert_eq!(refused(opening_of(None)), io::ErrorKind::InvalidInput);
    assert_eq!(
        refused(opening_of(Some(AuthContext::new(
            64,
            AuthScope::new([0; 32]),
            Arc::new(())
        )))),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn the_runtimes_helpers_stay_the_runtimes_whatever_context_an_opening_has() {
    // The legacy session endpoint is built for one operation's runtime and passes no context.
    let helpers = Helpers::new(helper("helper-runtime"), HelperSlots::new());
    let with = opening_of(Some(OperationAuth::context(helper("helper-a"), None)));
    assert_eq!(
        selected(&helpers, &with).unwrap(),
        PathBuf::from("helper-runtime")
    );
    assert_eq!(
        selected(&helpers, &opening_of(None)).unwrap(),
        PathBuf::from("helper-runtime")
    );
}
