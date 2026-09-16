//! The stub for targets with no copy-on-write call at all.

#[cfg(not(any(
    target_vendor = "apple",
    windows,
    all(
        target_os = "linux",
        not(any(target_arch = "sparc", target_arch = "sparc64"))
    )
)))]
pub(crate) mod imp {
    //! No native mechanism for this target: every copy is ordinary and says
    //! so once, through the copy-wide `NativeUnavailable` warning.
    //!
    //! What lands here is a target with no copy-on-write call this crate
    //! binds -- the BSDs, Solaris, SPARC Linux -- not a filesystem that cannot
    //! clone. A filesystem is never ruled out in advance: on the three
    //! platforms with a mechanism the operation decides, per file.

    use std::fs::File;
    use std::path::Path;

    use crate::NativeMechanism;
    use crate::native::Outcome;

    pub(crate) const MECHANISM: NativeMechanism = NativeMechanism::None;

    /// Never called: [`Plan::for_request`](crate::native::Plan::for_request) makes no
    /// attempt when [`MECHANISM`] is `None`. It exists so the platform seam
    /// has the same shape on every target.
    pub(crate) fn clone_regular_file(_source: &File, _temporary: &Path) -> Outcome {
        Outcome::Unsupported(
            "no native copy-on-write mechanism is compiled in for this target".to_owned(),
        )
    }
}
