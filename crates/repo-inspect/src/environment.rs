//! Invocation-level Git environment redirections.
//!
//! Design §4.0: "Reject invocation-level `GIT_ALTERNATE_OBJECT_DIRECTORIES`,
//! `GIT_OBJECT_DIRECTORY`, `GIT_COMMON_DIR`, `GIT_DIR`, or `GIT_WORK_TREE`
//! overrides for local create; do not let environment redirection bypass the
//! per-repository checks."
//!
//! The repository is opened without libgit2's `FROM_ENV` flag, so these
//! variables never silently change *what* is inspected. They are still
//! refused, because they would change what a later Git invocation in the same
//! environment operates on.
//!
//! The view is injectable so that a test — and a caller inspecting on behalf
//! of another invocation's environment — states it explicitly instead of
//! mutating process-global state.

use std::collections::BTreeSet;

use gwz_repo_contract::LayoutHazard;

/// The five variables design §4.0 names, in the order they are reported.
pub(crate) const REDIRECTING_VARIABLES: &[&str] = &[
    "GIT_DIR",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_WORK_TREE",
];

/// Which redirecting variables are set for this invocation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Environment {
    /// Read the process environment at each inspection.
    #[default]
    Process,
    /// An explicit view: exactly these redirecting variables are set.
    Explicit(BTreeSet<String>),
}

impl Environment {
    /// A view in which none of the redirecting variables is set.
    pub fn none() -> Self {
        Self::Explicit(BTreeSet::new())
    }

    /// A view built from an environment listing (`std::env::vars()` shape).
    /// Only the redirecting variables matter, and only when non-empty: Git
    /// treats an empty value as unset.
    pub fn from_vars<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let set = vars
            .into_iter()
            .filter(|(name, value)| {
                !value.as_ref().is_empty()
                    && REDIRECTING_VARIABLES.contains(&name.as_ref().to_ascii_uppercase().as_str())
            })
            .map(|(name, _)| name.as_ref().to_ascii_uppercase())
            .collect();
        Self::Explicit(set)
    }

    fn is_set(&self, variable: &str) -> bool {
        match self {
            Self::Process => std::env::var_os(variable).is_some_and(|value| !value.is_empty()),
            Self::Explicit(set) => set.contains(variable),
        }
    }

    /// One [`LayoutHazard::EnvironmentOverride`] per redirecting variable that
    /// is set, in a fixed order.
    pub(crate) fn overrides(&self) -> Vec<LayoutHazard> {
        REDIRECTING_VARIABLES
            .iter()
            .filter(|variable| self.is_set(variable))
            .map(|variable| LayoutHazard::EnvironmentOverride {
                variable: (*variable).to_owned(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_empty_view_reports_no_override() {
        assert!(Environment::none().overrides().is_empty());
    }

    #[test]
    fn every_redirecting_variable_is_reported_and_unrelated_ones_are_not() {
        let view = Environment::from_vars([
            ("GIT_DIR", "/elsewhere/.git"),
            ("GIT_COMMON_DIR", "/elsewhere"),
            ("GIT_OBJECT_DIRECTORY", "/objects"),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES", "/alt"),
            ("GIT_WORK_TREE", "/work"),
            ("GIT_AUTHOR_NAME", "someone"),
            ("PATH", "/usr/bin"),
        ]);
        let reported: Vec<_> = view
            .overrides()
            .into_iter()
            .map(|hazard| match hazard {
                LayoutHazard::EnvironmentOverride { variable } => variable,
                other => panic!("expected an environment override, got {other:?}"),
            })
            .collect();
        assert_eq!(reported, REDIRECTING_VARIABLES);
    }

    #[test]
    fn an_empty_value_is_unset_and_the_name_is_matched_case_insensitively() {
        assert!(
            Environment::from_vars([("GIT_DIR", "")])
                .overrides()
                .is_empty()
        );
        assert_eq!(
            Environment::from_vars([("git_dir", "/elsewhere")]).overrides(),
            vec![LayoutHazard::EnvironmentOverride {
                variable: "GIT_DIR".to_owned()
            }]
        );
    }
}
