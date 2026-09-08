use std::io;

use crate::filesystem::{FsFile, FsLockGuard};

pub(super) struct AdvisoryLock {
    file: FsFile,
    _guard: FsLockGuard,
}

impl AdvisoryLock {
    pub(super) fn try_acquire(file: FsFile) -> io::Result<Option<Self>> {
        let Some(guard) = file.filesystem().try_lock_file(&file)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            file,
            _guard: guard,
        }))
    }

    pub(super) fn file(&self) -> &FsFile {
        &self.file
    }
}
