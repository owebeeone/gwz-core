//! The artifact record types: the manifest, lock, snapshot and marker
//! shapes as they are serialized into `gwz.conf/`, with their validation.

use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManifestArtifact {
    pub schema: String,
    pub workspace: WorkspaceHeader,
    pub members: Vec<ManifestMember>,
}

impl ManifestArtifact {
    pub fn from_yaml(text: &str) -> ModelResult<Self> {
        let artifact: Self = parse_yaml(text)?;
        artifact.validate()?;
        Ok(artifact)
    }

    /// Serialize with the machine-managed banner. The banner is a YAML comment, so
    /// `from_yaml` is unaffected, and it rides on every write path because every write
    /// path goes through here.
    pub fn to_yaml(&self) -> ModelResult<String> {
        self.validate()?;
        Ok(format!("{CONF_BANNER}{}", emit_yaml(self)?))
    }

    pub fn validate(&self) -> ModelResult<()> {
        require_schema(&self.schema, WORKSPACE_SCHEMA)?;
        parse_id("workspace.id", "ws_", &self.workspace.id)?;

        let mut member_ids = BTreeSet::new();
        let mut active_paths = Vec::with_capacity(self.members.len());
        for member in &self.members {
            member.validate()?;
            if !member_ids.insert(member.id.as_str()) {
                return Err(invalid(format!("duplicate member id '{}'", member.id)));
            }
            if member.active {
                active_paths.push(MemberPath::parse(&member.path)?);
            }
        }
        crate::workspace::validate_member_path_set(&active_paths)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceHeader {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManifestMember {
    /// Access refusals for fresh clones may be quietly skipped; not remote visibility.
    #[serde(default, skip_serializing_if = "is_false")]
    pub private: bool,
    pub id: String,
    pub path: String,
    #[serde(rename = "type")]
    pub source_kind: ArtifactSourceKind,
    pub source_id: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desired: Option<DesiredRefArtifact>,
    pub remotes: Vec<RemoteArtifact>,
}

pub(super) fn is_false(value: &bool) -> bool {
    !*value
}

impl ManifestMember {
    fn validate(&self) -> ModelResult<()> {
        parse_id("member.id", "mem_", &self.id)?;
        parse_id("member.source_id", "src_", &self.source_id)?;
        MemberPath::parse(&self.path)?;
        if let Some(desired) = &self.desired {
            desired.validate()?;
        }
        reject_duplicate_remote_names(&self.remotes)?;
        for remote in &self.remotes {
            remote.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesiredRefArtifact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_only: Option<bool>,
}

impl DesiredRefArtifact {
    fn validate(&self) -> ModelResult<()> {
        if self.local_only == Some(false) {
            return Err(invalid("desired.local_only must be true when present"));
        }

        let mut targets = 0;
        targets += optional_text_target("desired.branch", &self.branch)?;
        targets += optional_text_target("desired.commit", &self.commit)?;
        targets += optional_text_target("desired.git_tag", &self.git_tag)?;
        if self.local_only == Some(true) {
            targets += 1;
        }

        if targets == 1 {
            Ok(())
        } else {
            Err(invalid("desired ref must specify exactly one target"))
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoteArtifact {
    pub name: String,
    pub url: String,
    pub fetch: bool,
    pub push: bool,
}

impl RemoteArtifact {
    fn validate(&self) -> ModelResult<()> {
        require_non_empty("remote.name", &self.name)?;
        require_non_empty("remote.url", &self.url)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LockArtifact {
    pub schema: String,
    pub workspace_id: String,
    pub manifest_schema: String,
    pub members: BTreeMap<String, ResolvedMemberArtifact>,
}

impl LockArtifact {
    pub fn from_yaml(text: &str) -> ModelResult<Self> {
        let artifact: Self = parse_yaml(text)?;
        artifact.validate()?;
        Ok(artifact)
    }

    /// Serialized WITHOUT the machine-managed banner that [`ManifestArtifact::to_yaml`]
    /// carries. The merge lane re-renders an accepted lock through a YAML value round trip
    /// that drops comments, and then requires the result to be byte-identical to the
    /// baseline it read off disk; a banner here would break that invariant for every
    /// no-op root merge. See `conf_integrity::CONF_BANNER`.
    pub fn to_yaml(&self) -> ModelResult<String> {
        self.validate()?;
        emit_yaml(self)
    }

    pub fn validate(&self) -> ModelResult<()> {
        require_schema(&self.schema, LOCK_SCHEMA)?;
        require_schema(&self.manifest_schema, WORKSPACE_SCHEMA)?;
        parse_id("workspace_id", "ws_", &self.workspace_id)?;
        for (member_id, member) in &self.members {
            parse_id("member id", "mem_", member_id)?;
            member.validate(true)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedMemberArtifact {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub source_kind: ArtifactSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detached: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub materialized: Option<bool>,
}

impl ResolvedMemberArtifact {
    pub(super) fn validate(&self, require_source_id: bool) -> ModelResult<()> {
        MemberPath::parse(&self.path)?;
        if require_source_id {
            let source_id = self
                .source_id
                .as_ref()
                .ok_or_else(|| invalid("resolved member source_id is required"))?;
            parse_id("member.source_id", "src_", source_id)?;
        } else if let Some(source_id) = &self.source_id {
            parse_id("member.source_id", "src_", source_id)?;
        }
        validate_optional_text("commit", &self.commit)?;
        validate_optional_text("branch", &self.branch)?;
        validate_optional_text("upstream", &self.upstream)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SnapshotArtifact {
    pub schema: String,
    pub workspace_id: String,
    pub snapshot_id: String,
    pub created_at: String,
    pub created_by: CreatedByArtifact,
    pub selected_members: Vec<String>,
    pub members: BTreeMap<String, ResolvedMemberArtifact>,
}

impl SnapshotArtifact {
    pub fn from_yaml(text: &str) -> ModelResult<Self> {
        let artifact: Self = parse_yaml(text)?;
        artifact.validate_for_read()?;
        Ok(artifact)
    }

    pub fn to_yaml(&self) -> ModelResult<String> {
        self.validate()?;
        emit_yaml(self)
    }

    pub fn validate(&self) -> ModelResult<()> {
        self.validate_for_read()?;
        validate_snapshot_id_for_creation(&self.snapshot_id)
    }

    fn validate_for_read(&self) -> ModelResult<()> {
        require_schema(&self.schema, SNAPSHOT_SCHEMA)?;
        parse_id("workspace_id", "ws_", &self.workspace_id)?;
        validate_snapshot_id_for_read(&self.snapshot_id)?;
        validate_member_record(
            &self.created_at,
            &self.created_by,
            &self.selected_members,
            &self.members,
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MarkerArtifact {
    pub schema: String,
    pub gwz_commit_id: String,
    pub workspace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_url_hash: Option<String>,
    pub created_at: String,
    pub created_by: CreatedByArtifact,
    pub root: MarkerRootArtifact,
    pub selected_targets: Vec<String>,
    pub committed_targets: Vec<String>,
    pub members: BTreeMap<String, ResolvedMemberArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<MarkerMergeArtifact>,
}

impl MarkerArtifact {
    pub fn from_yaml(text: &str) -> ModelResult<Self> {
        let artifact: Self = parse_yaml(text)?;
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn to_yaml(&self) -> ModelResult<String> {
        self.validate()?;
        emit_yaml(self)
    }

    pub fn validate(&self) -> ModelResult<()> {
        require_schema(&self.schema, MARKER_SCHEMA)?;
        require_uuid_v7("gwz_commit_id", &self.gwz_commit_id)?;
        parse_id("workspace_id", "ws_", &self.workspace_id)?;
        if let Some(hash) = &self.origin_url_hash {
            validate_origin_url_hash(hash)?;
        }
        self.root.validate()?;
        for target in &self.selected_targets {
            validate_target_ref("selected target", target)?;
        }
        for target in &self.committed_targets {
            validate_target_ref("committed target", target)?;
        }
        if let Some(merge) = &self.merge {
            if merge.selected_targets != self.selected_targets {
                return Err(invalid(
                    "marker selected_targets must match merge selected_targets",
                ));
            }
            merge.validate()?;
        }
        validate_member_record(&self.created_at, &self.created_by, &[], &self.members)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MarkerRootArtifact {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

impl MarkerRootArtifact {
    fn validate(&self) -> ModelResult<()> {
        if self.path != "." {
            return Err(invalid("marker root.path must be ."));
        }
        validate_optional_text("root.before_commit", &self.before_commit)?;
        validate_optional_text("root.branch", &self.branch)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreatedByArtifact {
    pub actor_id: String,
}

impl CreatedByArtifact {
    pub(super) fn validate(&self) -> ModelResult<()> {
        require_non_empty("created_by.actor_id", &self.actor_id)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactSourceKind {
    #[default]
    Git,
    Archive,
    Package,
    Local,
    Generated,
}
