
use crate::artifact::ArtifactSourceKind;



pub(crate) fn protocol_source_kind(source_kind: ArtifactSourceKind) -> crate::SourceKind {
    match source_kind {
        ArtifactSourceKind::Git => crate::SourceKind::Git,
        ArtifactSourceKind::Archive => crate::SourceKind::Archive,
        ArtifactSourceKind::Package => crate::SourceKind::Package,
        ArtifactSourceKind::Local => crate::SourceKind::Local,
        ArtifactSourceKind::Generated => crate::SourceKind::Generated,
    }
}

