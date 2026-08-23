use sha2::{Digest, Sha256};

use super::identity::DurableObjectIdentity;
use super::{CheckedArtifact, CheckedArtifactFact};

const MAGIC: &[u8; 8] = b"GWZCAUTH";
const VERSION: u16 = 1;
const MAX_RECORD_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ArtifactOperation {
    Replace,
    Remove,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RetainedSource {
    Missing,
    Existing(DurableObjectIdentity),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CheckedArtifactAuthority {
    pub(super) family_key: String,
    pub(super) action_key: String,
    pub(super) operation: ArtifactOperation,
    pub(super) canonical_path_identity: Vec<u8>,
    pub(super) artifact_root_identity: DurableObjectIdentity,
    pub(super) retained_parent_identity: DurableObjectIdentity,
    pub(super) expected: ExactValue,
    pub(super) goal: ExactValue,
    pub(super) retained_source: RetainedSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ExactValue {
    Missing,
    Bytes([u8; 32]),
}

impl ExactValue {
    fn from_fact(value: &CheckedArtifactFact) -> Option<Self> {
        match value {
            CheckedArtifactFact::Missing => Some(Self::Missing),
            CheckedArtifactFact::Bytes(bytes) => Some(Self::Bytes(Sha256::digest(bytes).into())),
            CheckedArtifactFact::Invalid => None,
        }
    }

    fn from_goal(goal: Option<&[u8]>) -> Self {
        goal.map_or(Self::Missing, |bytes| {
            Self::Bytes(Sha256::digest(bytes).into())
        })
    }

    fn encode(&self, output: &mut Vec<u8>) {
        match self {
            Self::Missing => output.push(0),
            Self::Bytes(digest) => {
                output.push(1);
                output.extend(digest);
            }
        }
    }

    fn decode(cursor: &mut Cursor<'_>) -> Option<Self> {
        match cursor.byte()? {
            0 => Some(Self::Missing),
            1 => Some(Self::Bytes(cursor.array()?)),
            _ => None,
        }
    }
}

impl CheckedArtifactAuthority {
    pub(super) fn for_source(
        artifact: &CheckedArtifact,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
        retained_parent_identity: DurableObjectIdentity,
        retained_source: RetainedSource,
    ) -> Option<Self> {
        let operation = if goal.is_some() {
            ArtifactOperation::Replace
        } else {
            ArtifactOperation::Remove
        };
        Some(Self {
            family_key: artifact.family_key(),
            action_key: artifact.action_key(expected, goal),
            operation,
            canonical_path_identity: artifact.canonical_path_identity.clone(),
            artifact_root_identity: artifact.root_identity.durable.clone(),
            retained_parent_identity,
            expected: ExactValue::from_fact(expected)?,
            goal: ExactValue::from_goal(goal),
            retained_source,
        })
    }

    pub(super) fn matches_request(
        &self,
        artifact: &CheckedArtifact,
        expected: &CheckedArtifactFact,
        goal: Option<&[u8]>,
    ) -> bool {
        self.family_key == artifact.family_key()
            && self.action_key == artifact.action_key(expected, goal)
            && self.canonical_path_identity == artifact.canonical_path_identity
            && self.artifact_root_identity == artifact.root_identity.durable
            && self.expected == ExactValue::from_fact(expected).unwrap_or(ExactValue::Missing)
            && self.goal == ExactValue::from_goal(goal)
            && self.operation
                == if goal.is_some() {
                    ArtifactOperation::Replace
                } else {
                    ArtifactOperation::Remove
                }
    }

    pub(super) fn encode(&self) -> Option<Vec<u8>> {
        let mut output = Vec::new();
        output.extend(MAGIC);
        output.extend(VERSION.to_le_bytes());
        put_string(&mut output, &self.family_key)?;
        put_string(&mut output, &self.action_key)?;
        output.push(match self.operation {
            ArtifactOperation::Replace => 1,
            ArtifactOperation::Remove => 2,
        });
        put_bytes(&mut output, &self.canonical_path_identity)?;
        put_bytes(&mut output, &self.artifact_root_identity.encode())?;
        put_bytes(&mut output, &self.retained_parent_identity.encode())?;
        self.expected.encode(&mut output);
        self.goal.encode(&mut output);
        match &self.retained_source {
            RetainedSource::Missing => output.push(0),
            RetainedSource::Existing(identity) => {
                output.push(1);
                put_bytes(&mut output, &identity.encode())?;
            }
        }
        (output.len() <= MAX_RECORD_BYTES).then_some(output)
    }

    pub(super) fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_RECORD_BYTES {
            return None;
        }
        let mut cursor = Cursor(bytes);
        if cursor.take(MAGIC.len())? != MAGIC || cursor.u16()? != VERSION {
            return None;
        }
        let family_key = cursor.string()?;
        let action_key = cursor.string()?;
        if !canonical_key(&family_key) || !canonical_key(&action_key) {
            return None;
        }
        let operation = match cursor.byte()? {
            1 => ArtifactOperation::Replace,
            2 => ArtifactOperation::Remove,
            _ => return None,
        };
        let canonical_path_identity = cursor.bytes()?.to_vec();
        if canonical_path_identity.len() > 4 * 1024 {
            return None;
        }
        let artifact_root_identity = DurableObjectIdentity::decode(cursor.bytes()?)?;
        let retained_parent_identity = DurableObjectIdentity::decode(cursor.bytes()?)?;
        let expected = ExactValue::decode(&mut cursor)?;
        let goal = ExactValue::decode(&mut cursor)?;
        let retained_source = match cursor.byte()? {
            0 => RetainedSource::Missing,
            1 => RetainedSource::Existing(DurableObjectIdentity::decode(cursor.bytes()?)?),
            _ => return None,
        };
        let authority = Self {
            family_key,
            action_key,
            operation,
            canonical_path_identity,
            artifact_root_identity,
            retained_parent_identity,
            expected,
            goal,
            retained_source,
        };
        if !cursor.done() || authority.encode().as_deref() != Some(bytes) {
            return None;
        }
        Some(authority)
    }
}

impl CheckedArtifact {
    pub(super) fn family_key(&self) -> String {
        let mut bytes = b"gwz.checked-artifact-family/v1\0".to_vec();
        bytes.extend(self.root_identity.durable.encode());
        bytes.extend((self.canonical_path_identity.len() as u32).to_le_bytes());
        bytes.extend(&self.canonical_path_identity);
        hex(&Sha256::digest(bytes))
    }

    pub(super) fn action_key(&self, expected: &CheckedArtifactFact, goal: Option<&[u8]>) -> String {
        let mut bytes = b"gwz.checked-artifact-action/v1\0".to_vec();
        bytes.extend(self.family_key().as_bytes());
        match expected {
            CheckedArtifactFact::Missing => bytes.push(0),
            CheckedArtifactFact::Bytes(value) => {
                bytes.push(1);
                bytes.extend(Sha256::digest(value));
            }
            CheckedArtifactFact::Invalid => bytes.push(2),
        }
        match goal {
            Some(value) => {
                bytes.push(1);
                bytes.extend(Sha256::digest(value));
            }
            None => bytes.push(0),
        }
        hex(&Sha256::digest(bytes))
    }
}

pub(super) fn authority_name(family: &str, action: &str) -> String {
    format!("ca1-{family}-{action}.authority")
}

pub(super) fn family_prefix(family: &str) -> String {
    format!("ca1-{family}-")
}

pub(super) fn goal_name(family: &str, action: &str, identity: &[u8; 16]) -> String {
    format!("ca1-{family}-{action}-{}.goal", hex(identity))
}

pub(super) fn source_name(family: &str, action: &str, identity: &[u8; 16]) -> String {
    format!("ca1-{family}-{action}-{}.source", hex(identity))
}

/// The write-ahead staging name for one kind of family record, in one action.
///
/// **Deterministic, and derived only from observed durable state** — the family
/// key (this workspace's root identity and the artifact's canonical path
/// identity) and the action key (the expected fact and goal digests). R2-D Phase
/// 4 Step 4.2 replaced the `getrandom` nonce this used to mint per attempt: plan
/// §4 Step 4.2 calls a random retry name "a standing violation of the R2 stop
/// clause **the moment it is on a successful converted path**", and Step 4.1 put
/// these two edges (E20 `ensure_goal`, E21 `publish_scratch`) on exactly such a
/// path. The stop clause's own wording is "retry reuses names/capacity, never a
/// nonce" (plan §4 Step 1.1).
///
/// The harm the nonce did was not abstract. The name is dotted, so
/// `inspect_family`'s `ca1-{family}-` filter skips it: a crash between the create
/// and the publication left an orphan that nothing could see, name, or reclaim —
/// one per crash, for ever. A resume now derives the *same* name from the same
/// durable state and reuses it.
///
/// **Action-scoped, not global.** Two drives collide only when they share a
/// family *and* an action, i.e. the same artifact with the same expected fact and
/// the same goal; different artifacts and different actions get different names
/// by construction. That is what makes the determinism safe without leaning on
/// the workspace mutator lock — though that lock does serialize gwz mutators in
/// separate processes (`operation/workspace_mutator_lock.rs`), and a same-action
/// collision already ended in a typed refusal before this change, because two
/// published goal aliases make `inspect_family` return `foreign`.
///
/// Dotted deliberately: staying outside the `ca1-{family}-` grammar means no
/// older gwz reading this private area sees a name it would classify as foreign.
pub(super) fn scratch_name(family: &str, action: &str, kind: &str) -> String {
    format!(".ca1-{family}-{action}-{kind}.scratch")
}

fn canonical_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn put_string(output: &mut Vec<u8>, value: &str) -> Option<()> {
    put_bytes(output, value.as_bytes())
}

fn put_bytes(output: &mut Vec<u8>, value: &[u8]) -> Option<()> {
    let length = u32::try_from(value.len()).ok()?;
    output.extend(length.to_le_bytes());
    output.extend(value);
    Some(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let (value, tail) = self.0.split_at_checked(length)?;
        self.0 = tail;
        Some(value)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let length = u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize;
        self.take(length)
    }

    fn string(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?.to_vec()).ok()
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }

    fn done(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_encoding_is_canonical_and_bounded() {
        let authority = CheckedArtifactAuthority {
            family_key: "1".repeat(64),
            action_key: "2".repeat(64),
            operation: ArtifactOperation::Replace,
            canonical_path_identity: vec![3, 4, 5],
            artifact_root_identity: DurableObjectIdentity::Windows {
                volume_guid: vec![6, 7],
                file_id: [8; 16],
            },
            retained_parent_identity: DurableObjectIdentity::Mac {
                volume_uuid: [9; 16],
                persistent_object_id: [10; 8],
            },
            expected: ExactValue::Bytes([11; 32]),
            goal: ExactValue::Bytes([12; 32]),
            retained_source: RetainedSource::Existing(DurableObjectIdentity::Linux {
                filesystem_id: vec![13; 8],
                handle_type: 14,
                file_handle: vec![15; 24],
            }),
        };
        let bytes = authority.encode().unwrap();
        assert_eq!(CheckedArtifactAuthority::decode(&bytes), Some(authority));
    }

    #[test]
    fn unknown_or_noncanonical_authority_is_rejected() {
        let mut bytes = Vec::from(*MAGIC);
        bytes.extend((VERSION + 1).to_le_bytes());
        assert!(CheckedArtifactAuthority::decode(&bytes).is_none());
    }
}
