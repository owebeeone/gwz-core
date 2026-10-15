//! A fixed-size allocation of units that is overwritten, spare capacity included, before it is freed: the owner
//! for the bytes of a helper's environment, which can carry credentials (`http.extraHeader`, a URL's password) in
//! the configuration view. It never grows, so no outgrown buffer that held part of it is freed unwiped. A unit is
//! a byte, or on Windows a UTF-16 unit.
//!
//! The probe is how a test sees the wipe: it records the owner's length and whether every unit was zero when the
//! owner dropped, as `https_worker/native.rs`'s `Header` does. A production owner carries the inert one.
use crate::session_host::environment::overwrite;

pub(super) struct Wiped<T: Copy + Default + PartialEq> {
    units: Vec<T>,
    probe: Probe,
}

impl<T: Copy + Default + PartialEq> Wiped<T> {
    /// `len` zero units, to be filled in place.
    pub(super) fn zeroed(len: usize, probe: Probe) -> Self {
        Self {
            units: vec![T::default(); len],
            probe,
        }
    }

    pub(super) fn as_slice(&self) -> &[T] {
        &self.units
    }

    pub(super) fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.units
    }
}

impl<T: Copy + Default + PartialEq> std::fmt::Debug for Wiped<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Wiped(REDACTED)")
    }
}

impl<T: Copy + Default + PartialEq> Drop for Wiped<T> {
    fn drop(&mut self) {
        overwrite(&mut self.units);
        self.probe.observe(&self.units);
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        use std::sync::{Arc, Mutex};

        /// What a test learns about each owner that dropped: its length and whether it was all zero.
        pub(super) type Records = Arc<Mutex<Vec<(usize, bool)>>>;

        #[derive(Clone, Default)]
        pub(super) struct Probe(Option<Records>);

        impl Probe {
            pub(super) fn inert() -> Self {
                Self::default()
            }

            pub(super) fn recording(records: &Records) -> Self {
                Self(Some(Arc::clone(records)))
            }

            fn observe<T: Copy + Default + PartialEq>(&self, units: &[T]) {
                if let Some(records) = &self.0 {
                    records
                        .lock()
                        .unwrap()
                        .push((units.len(), units.iter().all(|unit| *unit == T::default())));
                }
            }
        }
    } else {
        #[derive(Clone, Default)]
        pub(super) struct Probe;

        impl Probe {
            pub(super) fn inert() -> Self {
                Self
            }

            fn observe<T: Copy + Default + PartialEq>(&self, _units: &[T]) {}
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[test]
        fn an_owner_is_all_zero_and_as_long_as_it_was_when_it_drops() {
            let records = Records::default();
            let mut owner = Wiped::<u16>::zeroed(5, Probe::recording(&records));
            owner.as_mut_slice().copy_from_slice(&[1, 2, 3, 4, 5]);
            assert_eq!(owner.as_slice(), [1, 2, 3, 4, 5]);
            drop(owner);
            assert_eq!(*records.lock().unwrap(), vec![(5, true)]);
        }
    }
}
