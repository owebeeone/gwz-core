use super::*;

#[cfg(not(test))]
pub(crate) fn make_filesystem() -> impl FileSystem {
    native::NativeFileSystem
}

#[cfg(test)]
pub(crate) fn make_filesystem() -> TestFileSystem {
    if crate::test_backend::modes().fake_filesystem {
        TestFileSystem(Backend::Memory(fake::FakeFileSystem))
    } else {
        TestFileSystem(Backend::Native(native::NativeFileSystem))
    }
}

#[cfg(test)]
pub(crate) struct TestFileSystem(Backend);
#[cfg(test)]
enum Backend {
    Native(native::NativeFileSystem),
    Memory(fake::FakeFileSystem),
}
#[cfg(test)]
macro_rules! forward {
    ($name:ident($($arg:ident: $ty:ty),*) -> $result:ty) => {
        fn $name(&self, $($arg: $ty),*) -> $result {
            match &self.0 { Backend::Native(fs) => fs.$name($($arg),*), Backend::Memory(fs) => fs.$name($($arg),*) }
        }
    };
}
#[cfg(test)]
impl FileSystem for TestFileSystem {
    forward!(canonical_path(path: &Path) -> io::Result<std::path::PathBuf>);
    forward!(metadata(path: &Path) -> io::Result<FsMetadata>);
    forward!(metadata_at(parent: &FsDirectory, name: &OsStr) -> io::Result<FsMetadata>);
    forward!(link_target(path: &Path) -> io::Result<std::path::PathBuf>);
    forward!(remove_file_at(parent: &FsDirectory, name: &OsStr) -> io::Result<()>);
    forward!(remove_directory_at(parent: &FsDirectory, name: &OsStr) -> io::Result<()>);
    forward!(open_directory(path: &Path) -> io::Result<FsDirectory>);
    forward!(clone_directory(directory: &FsDirectory) -> io::Result<FsDirectory>);
    forward!(open_directory_at(parent: &FsDirectory, name: &OsStr) -> io::Result<FsDirectory>);
    forward!(create_directory_at(parent: &FsDirectory, name: &OsStr) -> io::Result<()>);
    forward!(open_file_at(parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile>);
    forward!(open_lock_file_at(parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile>);
    forward!(open_file_for_write_at(parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile>);
    forward!(create_file_at(parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile>);
    forward!(read_at(file: &FsFile, offset: u64, bytes: &mut [u8]) -> io::Result<usize>);
    forward!(file_len(file: &FsFile) -> io::Result<u64>);
    forward!(write_at(file: &FsFile, offset: u64, bytes: &[u8]) -> io::Result<usize>);
    forward!(set_len(file: &FsFile, length: u64) -> io::Result<()>);
    forward!(sync_file(file: &FsFile) -> io::Result<()>);
    forward!(directory_identity(directory: &FsDirectory) -> io::Result<FsIdentity>);
    forward!(file_identity(file: &FsFile) -> io::Result<FsIdentity>);
    forward!(rename(source: &Path, destination: &Path, mode: RenameMode) -> io::Result<()>);
    forward!(sync_directory(path: &Path) -> io::Result<()>);
    forward!(sync_directory_at(directory: &FsDirectory) -> io::Result<()>);
    forward!(read_directory(path: &Path) -> io::Result<Vec<FsDirectoryEntry>>);
    forward!(read_directory_at(directory: &FsDirectory) -> io::Result<Vec<FsDirectoryEntry>>);
    forward!(directory_entry_matches(parent: &FsDirectory, name: &OsStr, child: &FsDirectory) -> io::Result<bool>);
    forward!(file_entry_matches(parent: &FsDirectory, name: &OsStr, child: &FsFile) -> io::Result<bool>);
    forward!(test_create_symlink_at(parent: &FsDirectory, name: &OsStr, target: &Path) -> io::Result<()>);
    forward!(try_lock_file(file: &FsFile) -> io::Result<Option<FsLockGuard>>);
    forward!(rename_at(source: &FsDirectory, name: &OsStr, destination: &FsDirectory, target: &OsStr, mode: RenameMode) -> io::Result<()>);
    forward!(test_workspace() -> io::Result<TestFsWorkspace>);
}
