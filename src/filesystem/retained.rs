//! Ergonomic retained-handle operations. All effects go through FileSystem;
//! these adapters contain no native or memory implementation.
use super::*;
use std::io::{Read, Seek, SeekFrom, Write};

impl FsMetadata {
    pub(crate) fn dev(&self) -> u64 {
        self.identity.namespace
    }
    pub(crate) fn ino(&self) -> u64 {
        self.identity.object
    }
    pub(crate) fn len(&self) -> u64 {
        self.length
    }
}

impl FsDirectory {
    pub(crate) fn open_file(
        &self,
        name: impl AsRef<OsStr>,
        mode: &FsOpenMode,
    ) -> io::Result<FsFile> {
        let fs = make_filesystem();
        match mode {
            FsOpenMode::WriteOrCreate => match fs.open_file_for_write_at(self, name.as_ref()) {
                Ok(file) => Ok(file),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match fs.create_file_at(self, name.as_ref()) {
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            fs.open_file_for_write_at(self, name.as_ref())
                        }
                        result => result,
                    }
                }
                Err(error) => Err(error),
            },
            FsOpenMode::Read => fs.open_file_at(self, name.as_ref()),
            FsOpenMode::Write { create_new: true } => fs.create_file_at(self, name.as_ref()),
            FsOpenMode::Write { create_new: false } => {
                fs.open_file_for_write_at(self, name.as_ref())
            }
        }
    }
    pub(crate) fn entry_metadata(&self, path: impl AsRef<Path>) -> io::Result<FsMetadata> {
        let fs = make_filesystem();
        let path = path.as_ref();
        let mut components = path.components().peekable();
        let mut parent = fs.clone_directory(self)?;
        while let Some(part) = components.next() {
            let Component::Normal(name) = part else {
                return Err(io::ErrorKind::InvalidInput.into());
            };
            if components.peek().is_none() {
                return fs.metadata_at(&parent, name);
            }
            parent = fs.open_directory_at(&parent, name)?;
        }
        Err(io::ErrorKind::InvalidInput.into())
    }
    pub(crate) fn retained_child(&self, name: impl AsRef<OsStr>) -> io::Result<Self> {
        make_filesystem().open_directory_at(self, name.as_ref())
    }
    pub(crate) fn create_child(&self, name: impl AsRef<OsStr>) -> io::Result<()> {
        make_filesystem().create_directory_at(self, name.as_ref())
    }
    pub(crate) fn clone_handle(&self) -> io::Result<Self> {
        make_filesystem().clone_directory(self)
    }
    pub(crate) fn entries(&self) -> io::Result<FsDirectoryNames> {
        make_filesystem().directory_names(self)
    }
    pub(crate) fn remove_leaf(&self, name: impl AsRef<OsStr>) -> io::Result<()> {
        make_filesystem().remove_file_at(self, name.as_ref())
    }
    pub(crate) fn identify(&self) -> io::Result<FsIdentity> {
        make_filesystem().directory_identity(self)
    }
}
impl FsFile {
    pub(crate) fn metadata(&self) -> io::Result<FsMetadata> {
        make_filesystem().file_metadata(self)
    }
    pub(crate) fn sync_all(&self) -> io::Result<()> {
        make_filesystem().sync_file(self)
    }
    pub(crate) fn set_len(&self, len: u64) -> io::Result<()> {
        make_filesystem().set_len(self, len)
    }
}
impl Read for FsFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let read = make_filesystem().read_at(self, self.1, bytes)?;
        self.1 += read as u64;
        Ok(read)
    }
}
impl Write for FsFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = make_filesystem().write_at(self, self.1, bytes)?;
        self.1 += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for FsFile {
    fn seek(&mut self, seek: SeekFrom) -> io::Result<u64> {
        let position = match seek {
            SeekFrom::Start(value) => value,
            SeekFrom::Current(delta) => self
                .1
                .checked_add_signed(delta)
                .ok_or(io::ErrorKind::InvalidInput)?,
            SeekFrom::End(delta) => make_filesystem()
                .file_len(self)?
                .checked_add_signed(delta)
                .ok_or(io::ErrorKind::InvalidInput)?,
        };
        self.1 = position;
        Ok(position)
    }
}

pub(super) fn encode_path_identity(
    relative: &Path,
    mut component_identity: impl FnMut(&OsStr) -> io::Result<Vec<u8>>,
) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path identity contains a noncanonical component",
            ));
        };
        let mut bytes = component_identity(component)?;
        if bytes.len() > u16::MAX as usize {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path identity component is too long",
            ));
        }
        output.extend((bytes.len() as u16).to_le_bytes());
        output.append(&mut bytes);
    }
    if output.len() > 4 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "canonical path identity exceeds 4 KiB",
        ));
    }
    Ok(output)
}
