//! The values of one helper's environment, each in a wiping owner. The configuration view among them can carry a
//! credential (`http.extraHeader`, a URL's password), so no value is ever held in a plain `OsString` or `String`
//! on its way to the child: it is built once, at its final size, in the unit the platform's spawn wants (bytes on
//! Unix, UTF-16 on Windows), moved from owner to owner, and overwritten when the last one drops. What std's
//! `Command` copies on Unix is outside it (the helper-owner contract's OQ-C lists it).
use crate::git::endpoint::https_auth::wiped::{Probe, Wiped};
use std::ffi::OsStr;

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::os::unix::ffi::OsStrExt;

        /// What an environment value is made of: the bytes a POSIX environment holds.
        pub(in crate::git::endpoint::https_auth) type Unit = u8;

        impl EnvValue {
            /// `value`, copied once into an owner of its own.
            pub(in crate::git::endpoint::https_auth) fn from_os(value: &OsStr, probe: Probe) -> Self {
                let mut owner = Wiped::zeroed(value.len(), probe);
                owner.as_mut_slice().copy_from_slice(value.as_bytes());
                Self(owner)
            }

            /// The value as the spawn takes it.
            pub(in crate::git::endpoint::https_auth) fn as_os_str(&self) -> &OsStr {
                OsStr::from_bytes(self.units())
            }
        }
    } else if #[cfg(windows)] {
        use std::os::windows::ffi::OsStrExt;

        /// What an environment value is made of: the UTF-16 units a Windows environment block holds.
        pub(in crate::git::endpoint::https_auth) type Unit = u16;

        impl EnvValue {
            /// `value`, copied once into an owner of its own, sized before it is filled.
            pub(in crate::git::endpoint::https_auth) fn from_os(value: &OsStr, probe: Probe) -> Self {
                let mut owner = Wiped::zeroed(value.encode_wide().count(), probe);
                for (slot, unit) in owner.as_mut_slice().iter_mut().zip(value.encode_wide()) {
                    *slot = unit;
                }
                Self(owner)
            }
        }
    }
}

/// One variable's value, overwritten when it drops.
pub(in crate::git::endpoint::https_auth) struct EnvValue(Wiped<Unit>);

impl EnvValue {
    /// A value an owner of the platform's units already holds.
    pub(in crate::git::endpoint::https_auth) fn from_owner(owner: Wiped<Unit>) -> Self {
        Self(owner)
    }

    pub(in crate::git::endpoint::https_auth) fn units(&self) -> &[Unit] {
        self.0.as_slice()
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        impl EnvValue {
            /// A plain copy, for a comparison or a test's own child. Not for the product.
            pub(in crate::git::endpoint::https_auth) fn to_os_string(&self) -> std::ffi::OsString {
                cfg_if::cfg_if! {
                    if #[cfg(unix)] {
                        self.as_os_str().to_owned()
                    } else {
                        std::os::windows::ffi::OsStringExt::from_wide(self.units())
                    }
                }
            }
        }
    }
}
