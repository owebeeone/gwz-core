//! In-memory filesystem. There is no host I/O fallback.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub(super) struct FakeFileSystem;
#[derive(Clone)]
pub(super) struct Handle {
    tree: Arc<Mutex<Tree>>,
    id: u64,
    writable: bool,
}
pub(super) struct Lock {
    tree: Arc<Mutex<Tree>>,
    id: u64,
}
struct Tree {
    active: bool,
    next: u64,
    nodes: BTreeMap<u64, Node>,
    locks: BTreeSet<u64>,
}
enum Node {
    Directory(BTreeMap<OsString, u64>),
    File { bytes: Vec<u8>, durable: Vec<u8> },
    Symlink(PathBuf),
}
type Registry = Mutex<BTreeMap<PathBuf, Weak<Mutex<Tree>>>>;
fn registry() -> &'static Registry {
    static ROOTS: OnceLock<Registry> = OnceLock::new();
    ROOTS.get_or_init(Mutex::default)
}
fn error(kind: io::ErrorKind) -> io::Error {
    kind.into()
}
fn directory(value: &FsDirectory) -> io::Result<&Handle> {
    match &value.0 {
        DirectoryHandle::Memory(handle) => Ok(handle),
        _ => Err(error(io::ErrorKind::InvalidInput)),
    }
}
fn file(value: &FsFile) -> io::Result<&Handle> {
    match &value.0 {
        FileHandle::Memory(handle) => Ok(handle),
        _ => Err(error(io::ErrorKind::InvalidInput)),
    }
}
impl Tree {
    fn check(&self) -> io::Result<()> {
        if self.active {
            Ok(())
        } else {
            Err(error(io::ErrorKind::NotFound))
        }
    }
    fn entries(&self, id: u64) -> io::Result<&BTreeMap<OsString, u64>> {
        self.check()?;
        match self.nodes.get(&id) {
            Some(Node::Directory(entries)) => Ok(entries),
            _ => Err(error(io::ErrorKind::NotADirectory)),
        }
    }
    fn entries_mut(&mut self, id: u64) -> io::Result<&mut BTreeMap<OsString, u64>> {
        self.check()?;
        match self.nodes.get_mut(&id) {
            Some(Node::Directory(entries)) => Ok(entries),
            _ => Err(error(io::ErrorKind::NotADirectory)),
        }
    }
    fn lookup(&self, parent: u64, name: &OsStr) -> io::Result<u64> {
        self.entries(parent)?
            .get(name)
            .copied()
            .ok_or_else(|| error(io::ErrorKind::NotFound))
    }
    fn create(&mut self, parent: u64, name: &OsStr, node: Node) -> io::Result<u64> {
        if self.entries(parent)?.contains_key(name) {
            return Err(error(io::ErrorKind::AlreadyExists));
        }
        let id = self.next;
        self.next += 1;
        self.nodes.insert(id, node);
        self.entries_mut(parent)?.insert(name.to_owned(), id);
        Ok(id)
    }
    fn contains_directory(&self, ancestor: u64, candidate: u64) -> bool {
        if ancestor == candidate {
            return true;
        }
        match self.nodes.get(&ancestor) {
            Some(Node::Directory(entries)) => entries
                .values()
                .any(|id| self.contains_directory(*id, candidate)),
            _ => false,
        }
    }
}
fn memory_persistent_identity(identity: FsIdentity) -> FsObjectIdentity {
    let namespace = identity.namespace().to_be_bytes();
    let object = identity.object().wrapping_add(1).to_be_bytes();
    let mut volume = [0; 16];
    volume[..8].copy_from_slice(&namespace);
    volume[8..].copy_from_slice(&namespace);
    #[cfg(target_os = "macos")]
    let persistent = FsPersistentIdentity::Mac { volume, object };
    #[cfg(windows)]
    let persistent = {
        let mut file_id = [0; 16];
        file_id[..8].copy_from_slice(&object);
        file_id[8..].copy_from_slice(&object);
        FsPersistentIdentity::Windows {
            volume: vec![1],
            file_id,
        }
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let persistent = FsPersistentIdentity::Linux {
        volume,
        handle_type: 1,
        handle: object.to_vec(),
    };
    FsObjectIdentity {
        persistent,
        invocation: identity.encode().to_vec(),
    }
}

impl FileSystem for FakeFileSystem {
    fn legacy_directory_identity(&self, value: &FsDirectory) -> io::Result<FsLegacyObjectIdentity> {
        memory_legacy_identity(self.directory_identity(value)?)
    }
    fn legacy_file_identity(&self, value: &FsFile) -> io::Result<FsLegacyObjectIdentity> {
        memory_legacy_identity(self.file_identity(value)?)
    }
    fn legacy_rename_domain(&self, value: &FsDirectory) -> io::Result<FsLegacyRenameDomain> {
        let namespace = self.directory_identity(value)?.namespace();
        #[cfg(target_os = "linux")]
        return Ok(FsLegacyRenameDomain::LinuxMountId(namespace));
        #[cfg(target_os = "macos")]
        return Ok(FsLegacyRenameDomain::MacMountedFileSystem(
            namespace.to_be_bytes(),
        ));
        #[cfg(windows)]
        return Ok(FsLegacyRenameDomain::WindowsMountedVolume(
            namespace
                .to_be_bytes()
                .chunks_exact(2)
                .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                .collect(),
        ));
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        Err(io::ErrorKind::Unsupported.into())
    }
    fn legacy_path_identity(&self, value: &FsDirectory, relative: &Path) -> io::Result<Vec<u8>> {
        self.directory_identity(value)?;
        super::retained::encode_path_identity(relative, |component| {
            Ok(component.as_encoded_bytes().to_vec())
        })
    }
    fn directory_names(&self, directory: &FsDirectory) -> io::Result<FsDirectoryNames> {
        Ok(Box::new(
            self.read_directory_at(directory)?
                .into_iter()
                .map(|entry| Ok(entry.name)),
        ))
    }
    fn create_private_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        self.create_file_at(parent, name)
    }
    fn open_publication_source(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        component(name)?;
        let parent = directory(parent)?;
        let tree = parent.tree.lock().unwrap();
        let id = tree.lookup(parent.id, name)?;
        if !matches!(
            tree.nodes.get(&id),
            Some(Node::File { .. } | Node::Directory(_))
        ) {
            return Err(error(io::ErrorKind::InvalidInput));
        }
        Ok(FsFile(
            FileHandle::Memory(Handle {
                tree: Arc::clone(&parent.tree),
                id,
                writable: false,
            }),
            0,
        ))
    }
    fn publish_source(
        &self,
        source: FsPublicationSource<'_>,
        destination: &FsDirectory,
        target: &OsStr,
        mode: RenameMode,
        acquired: &dyn Fn() -> io::Result<()>,
    ) -> io::Result<()> {
        let _ = file(source.file)?;
        acquired()?;
        #[cfg(windows)]
        {
            // Windows publishes the captured object even if its old name has
            // been replaced since acquisition. No hard links are synthesized.
            let handle = file(source.file)?;
            let tree = handle.tree.lock().unwrap();
            tree.check()?;
            let (parent, name) = tree
                .nodes
                .iter()
                .find_map(|(id, node)| {
                    let Node::Directory(entries) = node else {
                        return None;
                    };
                    entries
                        .iter()
                        .find(|(_, child)| **child == handle.id)
                        .map(|(name, _)| (*id, name.clone()))
                })
                .ok_or(io::ErrorKind::NotFound)?;
            drop(tree);
            let parent = FsDirectory(DirectoryHandle::Memory(Handle {
                tree: Arc::clone(&handle.tree),
                id: parent,
                writable: false,
            }));
            return self.rename_at(&parent, &name, destination, target, mode);
        }
        #[cfg(not(windows))]
        self.rename_at(source.parent, source.name, destination, target, mode)
    }
    fn file_metadata(&self, value: &FsFile) -> io::Result<FsMetadata> {
        let handle = file(value)?;
        let tree = handle.tree.lock().unwrap();
        tree.check()?;
        let Some(Node::File { bytes, .. }) = tree.nodes.get(&handle.id) else {
            return Err(error(io::ErrorKind::InvalidInput));
        };
        Ok(FsMetadata {
            kind: FsKind::File,
            executable: false,
            identity: FsIdentity {
                namespace: Arc::as_ptr(&handle.tree) as usize as u64,
                object: handle.id,
            },
            length: bytes.len() as u64,
        })
    }
    fn support_profile(&self) -> FsSupportProfile {
        #[cfg(target_os = "macos")]
        return FsSupportProfile::MacPersistentId;
        #[cfg(windows)]
        return FsSupportProfile::WindowsFileId;
        #[cfg(not(any(target_os = "macos", windows)))]
        FsSupportProfile::LinuxPersistentHandle
    }
    fn persistent_directory_identity(
        &self,
        value: &FsDirectory,
    ) -> Result<FsObjectIdentity, FsProbeError> {
        self.directory_identity(value)
            .map(memory_persistent_identity)
            .map_err(|e| FsProbeError::io("identify memory directory", e))
    }
    fn persistent_file_identity(&self, value: &FsFile) -> Result<FsObjectIdentity, FsProbeError> {
        self.file_identity(value)
            .map(memory_persistent_identity)
            .map_err(|e| FsProbeError::io("identify memory file", e))
    }
    fn lookup_mode(&self, value: &FsDirectory) -> Result<FsLookupMode, FsProbeError> {
        self.directory_identity(value)
            .map(|_| FsLookupMode::Sensitive)
            .map_err(|e| FsProbeError::io("observe memory lookup mode", e))
    }
    fn rename_domain(&self, value: &FsDirectory) -> Result<Vec<u8>, FsProbeError> {
        self.directory_identity(value)
            .map(|id| id.namespace().to_be_bytes().to_vec())
            .map_err(|e| FsProbeError::io("identify memory rename domain", e))
    }
    fn describe_volume(&self, value: &FsDirectory) -> Result<FsVolumeDescription, FsProbeError> {
        self.directory_identity(value)
            .map(|_| FsVolumeDescription {
                name: Some("gwz-memory".into()),
                remote: false,
                volatile: false,
            })
            .map_err(|e| FsProbeError::io("describe memory volume", e))
    }
    fn canonical_path(&self, path: &Path) -> io::Result<PathBuf> {
        self.kind(path)?;
        // This initial profile has no symlink constructor. Do not resolve an
        // unsupported parent traversal by consulting the host filesystem.
        if path.components().any(|p| matches!(p, Component::ParentDir)) {
            return Err(error(io::ErrorKind::Unsupported));
        }
        Ok(path.components().collect())
    }
    fn metadata(&self, path: &Path) -> io::Result<FsMetadata> {
        if registry().lock().unwrap().contains_key(path) {
            let root = self.open_directory(path)?;
            return Ok(FsMetadata {
                kind: FsKind::Directory,
                executable: false,
                identity: self.directory_identity(&root)?,
                length: 0,
            });
        }
        let (parent, name) = split(path)?;
        let parent = self.open_directory(parent)?;
        let handle = directory(&parent)?;
        let tree = handle.tree.lock().unwrap();
        let id = tree.lookup(handle.id, name)?;
        let kind = match tree.nodes.get(&id) {
            Some(Node::Directory(_)) => FsKind::Directory,
            Some(Node::File { .. }) => FsKind::File,
            Some(Node::Symlink(_)) => FsKind::Symlink,
            None => return Err(error(io::ErrorKind::NotFound)),
        };
        Ok(FsMetadata {
            kind,
            executable: false,
            identity: FsIdentity {
                namespace: Arc::as_ptr(&handle.tree) as usize as u64,
                object: id,
            },
            length: match tree.nodes.get(&id) {
                Some(Node::File { bytes, .. }) => bytes.len() as u64,
                _ => 0,
            },
        })
    }
    fn metadata_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsMetadata> {
        component(name)?;
        let handle = directory(parent)?;
        let tree = handle.tree.lock().unwrap();
        let id = tree.lookup(handle.id, name)?;
        let kind = match tree.nodes.get(&id) {
            Some(Node::Directory(_)) => FsKind::Directory,
            Some(Node::File { .. }) => FsKind::File,
            Some(Node::Symlink(_)) => FsKind::Symlink,
            None => return Err(error(io::ErrorKind::NotFound)),
        };
        Ok(FsMetadata {
            kind,
            executable: false,
            identity: FsIdentity {
                namespace: Arc::as_ptr(&handle.tree) as usize as u64,
                object: id,
            },
            length: match tree.nodes.get(&id) {
                Some(Node::File { bytes, .. }) => bytes.len() as u64,
                _ => 0,
            },
        })
    }
    fn link_target(&self, path: &Path) -> io::Result<PathBuf> {
        let (parent, name) = split(path)?;
        let parent = self.open_directory(parent)?;
        let handle = directory(&parent)?;
        let tree = handle.tree.lock().unwrap();
        let id = tree.lookup(handle.id, name)?;
        match tree.nodes.get(&id) {
            Some(Node::Symlink(target)) => Ok(target.clone()),
            _ => Err(error(io::ErrorKind::InvalidInput)),
        }
    }
    fn remove_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<()> {
        component(name)?;
        let handle = directory(parent)?;
        let mut tree = handle.tree.lock().unwrap();
        let id = tree.lookup(handle.id, name)?;
        if matches!(tree.nodes.get(&id), Some(Node::Directory(_))) {
            return Err(error(io::ErrorKind::IsADirectory));
        }
        tree.entries_mut(handle.id)?.remove(name);
        Ok(())
    }
    fn remove_directory_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<()> {
        component(name)?;
        let handle = directory(parent)?;
        let mut tree = handle.tree.lock().unwrap();
        let id = tree.lookup(handle.id, name)?;
        match tree.nodes.get(&id) {
            Some(Node::Directory(entries)) if entries.is_empty() => {}
            Some(Node::Directory(_)) => return Err(error(io::ErrorKind::DirectoryNotEmpty)),
            Some(Node::File { .. } | Node::Symlink(_)) => {
                return Err(error(io::ErrorKind::NotADirectory));
            }
            None => return Err(error(io::ErrorKind::NotFound)),
        }
        tree.entries_mut(handle.id)?.remove(name);
        Ok(())
    }

    fn open_directory(&self, path: &Path) -> io::Result<FsDirectory> {
        // Match the longest registered fixture root; never consult host paths.
        let roots = registry().lock().unwrap();
        let (root, tree) = roots
            .iter()
            .filter(|(root, _)| path.starts_with(root))
            .max_by_key(|(root, _)| root.components().count())
            .ok_or_else(|| error(io::ErrorKind::NotFound))?;
        let shared = tree
            .upgrade()
            .ok_or_else(|| error(io::ErrorKind::NotFound))?;
        let tree = shared.lock().unwrap();
        tree.check()?;
        let mut id = 0;
        for part in path.strip_prefix(root).unwrap().components() {
            match part {
                Component::Normal(name) => id = tree.lookup(id, name)?,
                Component::CurDir => {}
                _ => return Err(error(io::ErrorKind::InvalidInput)),
            }
        }
        tree.entries(id)?;
        drop(tree);
        Ok(FsDirectory(DirectoryHandle::Memory(Handle {
            tree: shared,
            id,
            writable: false,
        })))
    }
    fn clone_directory(&self, value: &FsDirectory) -> io::Result<FsDirectory> {
        Ok(FsDirectory(DirectoryHandle::Memory(
            directory(value)?.clone(),
        )))
    }
    fn open_directory_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsDirectory> {
        component(name)?;
        let parent = directory(parent)?;
        let tree = parent.tree.lock().unwrap();
        let id = tree.lookup(parent.id, name)?;
        tree.entries(id)?;
        Ok(FsDirectory(DirectoryHandle::Memory(Handle {
            id,
            ..parent.clone()
        })))
    }
    fn create_directory_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<()> {
        component(name)?;
        let parent = directory(parent)?;
        parent
            .tree
            .lock()
            .unwrap()
            .create(parent.id, name, Node::Directory(BTreeMap::new()))?;
        Ok(())
    }
    fn open_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        component(name)?;
        let parent = directory(parent)?;
        let tree = parent.tree.lock().unwrap();
        let id = tree.lookup(parent.id, name)?;
        if !matches!(tree.nodes.get(&id), Some(Node::File { .. })) {
            return Err(error(io::ErrorKind::InvalidInput));
        }
        Ok(FsFile(
            FileHandle::Memory(Handle {
                id,
                writable: false,
                tree: parent.tree.clone(),
            }),
            0,
        ))
    }
    fn open_lock_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        self.open_file_at(parent, name)
    }
    fn open_file_for_write_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        component(name)?;
        let parent = directory(parent)?;
        let tree = parent.tree.lock().unwrap();
        let id = tree.lookup(parent.id, name)?;
        if !matches!(tree.nodes.get(&id), Some(Node::File { .. })) {
            return Err(error(io::ErrorKind::InvalidInput));
        }
        Ok(FsFile(
            FileHandle::Memory(Handle {
                id,
                writable: true,
                tree: parent.tree.clone(),
            }),
            0,
        ))
    }
    fn create_file_at(&self, parent: &FsDirectory, name: &OsStr) -> io::Result<FsFile> {
        component(name)?;
        let parent = directory(parent)?;
        let id = parent.tree.lock().unwrap().create(
            parent.id,
            name,
            Node::File {
                bytes: Vec::new(),
                durable: Vec::new(),
            },
        )?;
        Ok(FsFile(
            FileHandle::Memory(Handle {
                id,
                writable: true,
                tree: parent.tree.clone(),
            }),
            0,
        ))
    }
    fn read_at(&self, value: &FsFile, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let handle = file(value)?;
        let tree = handle.tree.lock().unwrap();
        tree.check()?;
        let Some(Node::File { bytes, .. }) = tree.nodes.get(&handle.id) else {
            return Err(error(io::ErrorKind::InvalidInput));
        };
        let offset = usize::try_from(offset).map_err(|_| error(io::ErrorKind::InvalidInput))?;
        let tail = bytes.get(offset..).unwrap_or_default();
        let count = tail.len().min(buffer.len());
        buffer[..count].copy_from_slice(&tail[..count]);
        Ok(count)
    }
    fn file_len(&self, value: &FsFile) -> io::Result<u64> {
        let handle = file(value)?;
        let tree = handle.tree.lock().unwrap();
        tree.check()?;
        match tree.nodes.get(&handle.id) {
            Some(Node::File { bytes, .. }) => {
                u64::try_from(bytes.len()).map_err(|_| error(io::ErrorKind::InvalidData))
            }
            _ => Err(error(io::ErrorKind::InvalidInput)),
        }
    }
    fn write_at(&self, value: &FsFile, offset: u64, buffer: &[u8]) -> io::Result<usize> {
        let handle = file(value)?;
        if !handle.writable {
            return Err(error(io::ErrorKind::PermissionDenied));
        }
        let mut tree = handle.tree.lock().unwrap();
        tree.check()?;
        let Some(Node::File { bytes, .. }) = tree.nodes.get_mut(&handle.id) else {
            return Err(error(io::ErrorKind::InvalidInput));
        };
        let offset = usize::try_from(offset).map_err(|_| error(io::ErrorKind::InvalidInput))?;
        let end = offset
            .checked_add(buffer.len())
            .ok_or_else(|| error(io::ErrorKind::InvalidInput))?;
        if buffer.is_empty() {
            return Ok(0);
        }
        if end > bytes.len() {
            bytes
                .try_reserve(end - bytes.len())
                .map_err(|_| error(io::ErrorKind::OutOfMemory))?;
            bytes.resize(end, 0);
        }
        bytes[offset..end].copy_from_slice(buffer);
        Ok(buffer.len())
    }
    fn set_len(&self, value: &FsFile, length: u64) -> io::Result<()> {
        let handle = file(value)?;
        if !handle.writable {
            return Err(error(io::ErrorKind::PermissionDenied));
        }
        let length = usize::try_from(length).map_err(|_| error(io::ErrorKind::InvalidInput))?;
        let mut tree = handle.tree.lock().unwrap();
        tree.check()?;
        let Some(Node::File { bytes, .. }) = tree.nodes.get_mut(&handle.id) else {
            return Err(error(io::ErrorKind::InvalidInput));
        };
        bytes.resize(length, 0);
        Ok(())
    }
    fn sync_file(&self, value: &FsFile) -> io::Result<()> {
        let handle = file(value)?;
        let mut tree = handle.tree.lock().unwrap();
        tree.check()?;
        let Some(Node::File { bytes, durable }) = tree.nodes.get_mut(&handle.id) else {
            return Err(error(io::ErrorKind::InvalidInput));
        };
        durable.clone_from(bytes);
        Ok(())
    }
    fn directory_identity(&self, value: &FsDirectory) -> io::Result<FsIdentity> {
        identity(directory(value)?)
    }
    fn file_identity(&self, value: &FsFile) -> io::Result<FsIdentity> {
        identity(file(value)?)
    }
    fn rename(&self, source: &Path, destination: &Path, mode: RenameMode) -> io::Result<()> {
        let (source_parent, source_name) = split(source)?;
        let (destination_parent, destination_name) = split(destination)?;
        let source_parent = self.open_directory(source_parent)?;
        let destination_parent = self.open_directory(destination_parent)?;
        rename_handles(
            &source_parent,
            source_name,
            &destination_parent,
            destination_name,
            mode,
        )
    }
    fn sync_directory(&self, path: &Path) -> io::Result<()> {
        self.open_directory(path).map(|_| ())
    }
    fn sync_directory_at(&self, value: &FsDirectory) -> io::Result<()> {
        let handle = directory(value)?;
        handle.tree.lock().unwrap().entries(handle.id).map(|_| ())
    }
    fn read_directory(&self, path: &Path) -> io::Result<Vec<FsDirectoryEntry>> {
        let opened = self.open_directory(path)?;
        self.read_directory_at(&opened)
    }
    fn read_directory_at(&self, value: &FsDirectory) -> io::Result<Vec<FsDirectoryEntry>> {
        let handle = directory(value)?;
        let tree = handle.tree.lock().unwrap();
        tree.entries(handle.id)?
            .iter()
            .map(|(name, id)| {
                let kind = match tree.nodes.get(id) {
                    Some(Node::Directory(_)) => FsKind::Directory,
                    Some(Node::File { .. }) => FsKind::File,
                    Some(Node::Symlink(_)) => FsKind::Symlink,
                    None => return Err(error(io::ErrorKind::NotFound)),
                };
                Ok(FsDirectoryEntry {
                    name: name.clone(),
                    kind,
                })
            })
            .collect()
    }
    fn directory_entry_matches(
        &self,
        parent: &FsDirectory,
        name: &OsStr,
        child: &FsDirectory,
    ) -> io::Result<bool> {
        component(name)?;
        let parent = directory(parent)?;
        let child = directory(child)?;
        if !Arc::ptr_eq(&parent.tree, &child.tree) {
            return Ok(false);
        }
        let tree = parent.tree.lock().unwrap();
        match tree.lookup(parent.id, name) {
            Ok(id) => Ok(id == child.id && matches!(tree.nodes.get(&id), Some(Node::Directory(_)))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn file_entry_matches(
        &self,
        parent: &FsDirectory,
        name: &OsStr,
        child: &FsFile,
    ) -> io::Result<bool> {
        component(name)?;
        let parent = directory(parent)?;
        let child = file(child)?;
        if !Arc::ptr_eq(&parent.tree, &child.tree) {
            return Ok(false);
        }
        let tree = parent.tree.lock().unwrap();
        match tree.lookup(parent.id, name) {
            Ok(id) => Ok(id == child.id && matches!(tree.nodes.get(&id), Some(Node::File { .. }))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn test_create_symlink_at(
        &self,
        parent: &FsDirectory,
        name: &OsStr,
        target: &Path,
    ) -> io::Result<()> {
        component(name)?;
        let parent = directory(parent)?;
        parent
            .tree
            .lock()
            .unwrap()
            .create(parent.id, name, Node::Symlink(target.to_path_buf()))?;
        Ok(())
    }
    fn try_lock_file(&self, value: &FsFile) -> io::Result<Option<FsLockGuard>> {
        let handle = file(value)?;
        let mut tree = handle.tree.lock().unwrap();
        tree.check()?;
        if !matches!(tree.nodes.get(&handle.id), Some(Node::File { .. })) {
            return Err(error(io::ErrorKind::InvalidInput));
        }
        if !tree.locks.insert(handle.id) {
            return Ok(None);
        }
        Ok(Some(FsLockGuard {
            _handle: LockHandle::Memory {
                _lock: Lock {
                    tree: handle.tree.clone(),
                    id: handle.id,
                },
            },
        }))
    }
    fn rename_at(
        &self,
        source: &FsDirectory,
        name: &OsStr,
        destination: &FsDirectory,
        target: &OsStr,
        mode: RenameMode,
    ) -> io::Result<()> {
        rename_handles(source, name, destination, target, mode)
    }
    fn test_workspace(&self) -> io::Result<TestFsWorkspace> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = if cfg!(windows) {
            "C:\\__gwz_memory__"
        } else {
            "/__gwz_memory__"
        };
        let path = PathBuf::from(base).join(NEXT.fetch_add(1, Ordering::Relaxed).to_string());
        let tree = Arc::new(Mutex::new(Tree {
            active: true,
            next: 1,
            nodes: BTreeMap::from([(0, Node::Directory(BTreeMap::new()))]),
            locks: BTreeSet::new(),
        }));
        registry()
            .lock()
            .unwrap()
            .insert(path.clone(), Arc::downgrade(&tree));
        let cleanup_path = path.clone();
        Ok(TestFsWorkspace {
            path,
            cleanup: Some(Box::new(move || {
                // Same lock order as open_directory; fixtures never reset other roots.
                registry().lock().unwrap().remove(&cleanup_path);
                tree.lock().unwrap().active = false;
            })),
        })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        self.tree.lock().unwrap().locks.remove(&self.id);
    }
}

fn identity(handle: &Handle) -> io::Result<FsIdentity> {
    handle.tree.lock().unwrap().check()?;
    Ok(FsIdentity {
        namespace: Arc::as_ptr(&handle.tree) as usize as u64,
        object: handle.id,
    })
}

fn rename_handles(
    source: &FsDirectory,
    name: &OsStr,
    destination: &FsDirectory,
    target: &OsStr,
    mode: RenameMode,
) -> io::Result<()> {
    component(name)?;
    component(target)?;
    let source = directory(source)?;
    let destination = directory(destination)?;
    if !Arc::ptr_eq(&source.tree, &destination.tree) {
        return Err(error(io::ErrorKind::CrossesDevices));
    }
    let mut tree = source.tree.lock().unwrap();
    let id = tree.lookup(source.id, name)?;
    let old = tree.entries(destination.id)?.get(target).copied();
    if old == Some(id) {
        return Ok(());
    }
    if old.is_some() && mode == RenameMode::NoReplace {
        return Err(error(io::ErrorKind::AlreadyExists));
    }
    if tree.contains_directory(id, destination.id) {
        return Err(error(io::ErrorKind::InvalidInput));
    }
    if let Some(old) = old {
        match (tree.nodes.get(&id), tree.nodes.get(&old)) {
            (Some(Node::File { .. } | Node::Symlink(_)), Some(Node::Directory(_))) => {
                return Err(error(io::ErrorKind::IsADirectory));
            }
            (Some(Node::Directory(_)), Some(Node::File { .. } | Node::Symlink(_))) => {
                return Err(error(io::ErrorKind::NotADirectory));
            }
            (_, Some(Node::Directory(entries))) if !entries.is_empty() => {
                return Err(error(io::ErrorKind::DirectoryNotEmpty));
            }
            _ => {}
        }
    }
    tree.entries_mut(source.id)?.remove(name);
    tree.entries_mut(destination.id)?
        .insert(target.to_owned(), id);
    Ok(())
}

fn memory_legacy_identity(identity: FsIdentity) -> std::io::Result<FsLegacyObjectIdentity> {
    let namespace = identity.namespace();
    let object = identity.object();
    #[cfg(not(windows))]
    let invocation = FsLegacyInvocationIdentity::Unix {
        device: namespace,
        inode: object,
    };
    #[cfg(target_os = "linux")]
    let durable = FsLegacyDurableIdentity::Linux {
        filesystem_id: namespace.to_be_bytes().to_vec(),
        handle_type: 1,
        file_handle: object.to_be_bytes().to_vec(),
    };
    #[cfg(target_os = "macos")]
    let durable = {
        let mut volume_uuid = [0; 16];
        volume_uuid[..8].copy_from_slice(&namespace.to_be_bytes());
        volume_uuid[8..].copy_from_slice(&namespace.to_be_bytes());
        FsLegacyDurableIdentity::Mac {
            volume_uuid,
            persistent_object_id: object.to_be_bytes(),
        }
    };
    #[cfg(windows)]
    let (durable, invocation) = {
        let volume_guid = namespace
            .to_be_bytes()
            .chunks_exact(2)
            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();
        let mut file_id = [0; 16];
        file_id[..8].copy_from_slice(&object.to_be_bytes());
        file_id[8..].copy_from_slice(&object.to_be_bytes());
        (
            FsLegacyDurableIdentity::Windows {
                volume_guid: volume_guid.clone(),
                file_id,
            },
            FsLegacyInvocationIdentity::Windows {
                volume_guid,
                file_id,
            },
        )
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    return Err(std::io::ErrorKind::Unsupported.into());
    Ok(FsLegacyObjectIdentity {
        durable,
        invocation,
    })
}
