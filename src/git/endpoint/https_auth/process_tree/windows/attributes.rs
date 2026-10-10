//! The attribute list of one `CreateProcessW`: the job the process is created in and the handles it inherits.
//! `UpdateProcThreadAttribute` keeps references to its values until the list is deleted, so they live in boxes
//! that do not move.
use std::io;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::Threading::{
        DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
        LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        PROC_THREAD_ATTRIBUTE_JOB_LIST, UpdateProcThreadAttribute,
    },
};

pub(super) struct Attributes {
    storage: Box<[usize]>,
    jobs: Box<[HANDLE; 1]>,
    handles: Box<[HANDLE; 3]>,
    initialized: bool,
}

impl Attributes {
    /// The process is created in `job` and inherits `handles` and nothing else. There is no fallback to creating
    /// the process outside the job or to inheriting everything.
    pub(super) fn new(job: &OwnedHandle, handles: [HANDLE; 3]) -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: sizing probe, null output and initialized size.
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut size);
        }
        if size == 0 || size > 65536 {
            return Err(io::Error::other(
                "the attribute list has an unexpected size",
            ));
        }
        let mut list = Self {
            storage: vec![0usize; size.div_ceil(size_of::<usize>())].into_boxed_slice(),
            jobs: Box::new([job.as_raw_handle()]),
            handles: Box::new(handles),
            initialized: false,
        };
        // SAFETY: the allocation is aligned, fixed and at least `size` bytes.
        if unsafe { InitializeProcThreadAttributeList(list.pointer(), 2, 0, &mut size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        list.initialized = true;
        // SAFETY: both boxed arrays stay live and unmoved until the list is deleted.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                list.pointer(),
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                list.jobs.as_ptr().cast(),
                size_of_val(list.jobs.as_ref()),
                std::ptr::null_mut(),
                std::ptr::null(),
            ) != 0
                && UpdateProcThreadAttribute(
                    list.pointer(),
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    list.handles.as_ptr().cast(),
                    size_of_val(list.handles.as_ref()),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                ) != 0
        };
        if !updated {
            return Err(io::Error::last_os_error());
        }
        Ok(list)
    }

    pub(super) fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: the list and both arrays it references are still live; field destructors run after this.
            unsafe {
                DeleteProcThreadAttributeList(self.pointer());
            }
        }
    }
}
