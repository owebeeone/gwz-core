//! The signature buffer's allocator (plan step 3.4, design U11).
//!
//! libssh2 frees the signature that the callback returns, with the `free` of the allocator its session was made with
//! (`Session::new` leaves it the default, the C library's). The callback allocates with `allocate`. If a Windows build
//! linked the two to different C runtimes, a block would be freed by a heap that did not make it, which corrupts the
//! heap. So the test hands libssh2 the blocks `allocate` made, a thousand times, and has libssh2 free them through its
//! own compiled `free`, then asks the C runtime and the process heap whether they are intact. The login rows beside it
//! (`ssh_tests::agent_auth`) take the same path through a real server.
use super::*;

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapValidate};

        unsafe extern "C" {
            /// The C runtime's heap check: `_HEAPOK` (-2) when the heap it owns is intact.
            fn _heapchk() -> c_int;
        }

        /// Fails unless both the C runtime's heap and the process heap are intact.
        fn assert_heaps_intact() {
            // SAFETY: both calls take no pointer that must be valid.
            let (runtime, process) = unsafe { (_heapchk(), HeapValidate(GetProcessHeap(), 0, ptr::null())) };
            assert_eq!(runtime, -2, "the C runtime's heap is damaged (_heapchk)");
            assert_ne!(process, 0, "the process heap is damaged (HeapValidate)");
        }
    } else {
        /// The C library's `free` is the one every block went through; there is no second heap to check.
        fn assert_heaps_intact() {}
    }
}

#[test]
fn libssh2_frees_a_thousand_signature_buffers_that_allocate_made() {
    let session = ssh2::Session::new().unwrap();
    for round in 0..1000_usize {
        let len = 1 + (round * 37) % 700;
        let memory = allocate(len);
        assert!(!memory.is_null(), "allocation {round} of {len} bytes");
        // SAFETY: the block has `len` bytes and no owner but this test; libssh2 takes ownership, as it does of the
        // signature that `sign` returns, and frees it with its session's allocator.
        unsafe {
            ptr::write_bytes(memory, (round % 251) as u8, len);
            let mut raw = session.raw();
            libssh2_sys::libssh2_free(&mut *raw, memory.cast());
        }
    }
    assert_heaps_intact();
}

#[test]
fn the_buffer_holds_what_it_was_given() {
    let memory = allocate(5);
    assert!(!memory.is_null());
    // SAFETY: five bytes were allocated; the block is freed once, by libssh2.
    unsafe {
        ptr::copy_nonoverlapping(b"sign!".as_ptr(), memory, 5);
        assert_eq!(slice::from_raw_parts(memory, 5), b"sign!");
        let session = ssh2::Session::new().unwrap();
        let mut raw = session.raw();
        libssh2_sys::libssh2_free(&mut *raw, memory.cast());
    }
}
