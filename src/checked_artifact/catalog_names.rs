//! Single owner for the fixed checked-artifact catalog/private-domain names.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogPrivateRootV1 {
    Workspace,
    GitDirectory,
}

impl CatalogPrivateRootV1 {
    const fn prefix(self) -> &'static [u8] {
        match self {
            Self::Workspace => b".gwz",
            Self::GitDirectory => b"gwz",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CatalogPrivateNameV1 {
    Final,
    BootstrapScratch,
    BootstrapActive,
    BootstrapStaging,
}

impl CatalogPrivateNameV1 {
    pub(in crate::checked_artifact) const ALL: &'static [Self] = &[
        Self::Final,
        Self::BootstrapScratch,
        Self::BootstrapActive,
        Self::BootstrapStaging,
    ];

    pub(in crate::checked_artifact) const fn leaf_bytes(self) -> &'static [u8] {
        match self {
            Self::Final => b"checked-artifacts",
            Self::BootstrapScratch => b"checked-artifacts-catalog-bootstrap-v1.scratch",
            Self::BootstrapActive => b"checked-artifacts-catalog-bootstrap-v1.active",
            Self::BootstrapStaging => b"checked-artifacts-catalog-bootstrap-v1.staging",
        }
    }

    pub(in crate::checked_artifact) fn relative_bytes(self, root: CatalogPrivateRootV1) -> Vec<u8> {
        let mut value = Vec::with_capacity(root.prefix().len() + 1 + self.leaf_bytes().len());
        value.extend_from_slice(root.prefix());
        value.push(b'/');
        value.extend_from_slice(self.leaf_bytes());
        value
    }

    pub(in crate::checked_artifact) fn relative_path(self, root: CatalogPrivateRootV1) -> PathBuf {
        let prefix = match root {
            CatalogPrivateRootV1::Workspace => ".gwz",
            CatalogPrivateRootV1::GitDirectory => "gwz",
        };
        PathBuf::from(prefix).join(
            std::str::from_utf8(self.leaf_bytes()).expect("fixed catalog name is valid ASCII"),
        )
    }
}
