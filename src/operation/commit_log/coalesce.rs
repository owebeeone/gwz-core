use super::CommitLogEntry;

const HEURISTIC_WINDOW_SECONDS: u64 = 10;
const GWZ_COMMIT_ID: &[u8] = b"GWZ-Commit-ID";
const GWZ_COMMIT_ID_LINE: &[u8] = b"GWZ-Commit-ID: ";
const GWZ_WORKSPACE_ID_LINE: &[u8] = b"GWZ-Workspace-ID: ";
const GWZ_ORIGIN_URL_HASH_LINE: &[u8] = b"GWZ-Origin-URL-Hash: sha256:";

/// The admission window owned by the S2.5 cross-cursor merge.
pub(super) const COALESCING_WINDOW_SECONDS: i64 = 60;

/// Why the commits in one workspace-level entry were assembled together.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CommitLogProvenance {
    None,
    Heuristic,
    Marker(String),
    MarkerInvalid,
}

/// A group assembled from candidates already admitted by the streaming merge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CommitLogGroup {
    entries: Vec<CommitLogEntry>,
    provenance: CommitLogProvenance,
    ordering_timestamp_seconds: i64,
}

impl CommitLogGroup {
    pub(super) fn entries(&self) -> &[CommitLogEntry] {
        &self.entries
    }

    pub(super) fn provenance(&self) -> &CommitLogProvenance {
        &self.provenance
    }

    pub(super) fn ordering_timestamp_seconds(&self) -> i64 {
        self.ordering_timestamp_seconds
    }
}

/// Assemble one finite candidate set into workspace-level entries.
///
/// S2.5 supplies candidates admitted by [`COALESCING_WINDOW_SECONDS`] and owns
/// all cursor positions, buffering, and closure. This function consumes its
/// input and retains no state between calls.
pub(super) fn assemble_commit_log_groups(
    entries: impl IntoIterator<Item = CommitLogEntry>,
    coalesce: bool,
) -> Vec<CommitLogGroup> {
    if !coalesce {
        return entries
            .into_iter()
            .map(|entry| {
                let provenance = match marker_identity(&entry.message) {
                    MarkerIdentity::Unusable => CommitLogProvenance::MarkerInvalid,
                    MarkerIdentity::Unmarked | MarkerIdentity::Key(_) => CommitLogProvenance::None,
                };
                finished_group(vec![entry], provenance)
            })
            .collect();
    }

    let mut groups = Vec::<PendingGroup>::new();
    for entry in entries {
        match marker_identity(&entry.message) {
            MarkerIdentity::Key(marker) => {
                if let Some(group) = groups.iter_mut().find(|group| {
                    group.marker_key() == Some(marker.as_str()) && group.accepts_repository(&entry)
                }) {
                    group.entries.push(entry);
                } else {
                    groups.push(PendingGroup::marker(entry, marker));
                }
            }
            MarkerIdentity::Unmarked => {
                if let Some(group) = groups
                    .iter_mut()
                    .find(|group| group.accepts_heuristic(&entry))
                {
                    group.entries.push(entry);
                } else {
                    groups.push(PendingGroup::unmarked(entry));
                }
            }
            MarkerIdentity::Unusable => groups.push(PendingGroup::opaque(entry)),
        }
    }

    groups.into_iter().map(PendingGroup::finish).collect()
}

#[derive(Debug)]
enum PendingKind {
    Marker(String),
    Unmarked,
    Opaque,
}

#[derive(Debug)]
struct PendingGroup {
    entries: Vec<CommitLogEntry>,
    kind: PendingKind,
}

impl PendingGroup {
    fn marker(entry: CommitLogEntry, marker: String) -> Self {
        Self {
            entries: vec![entry],
            kind: PendingKind::Marker(marker),
        }
    }

    fn unmarked(entry: CommitLogEntry) -> Self {
        Self {
            entries: vec![entry],
            kind: PendingKind::Unmarked,
        }
    }

    fn opaque(entry: CommitLogEntry) -> Self {
        Self {
            entries: vec![entry],
            kind: PendingKind::Opaque,
        }
    }

    fn marker_key(&self) -> Option<&str> {
        match &self.kind {
            PendingKind::Marker(marker) => Some(marker),
            PendingKind::Unmarked | PendingKind::Opaque => None,
        }
    }

    fn accepts_repository(&self, entry: &CommitLogEntry) -> bool {
        self.entries
            .iter()
            .all(|sibling| sibling.target.member_id != entry.target.member_id)
    }

    fn accepts_heuristic(&self, entry: &CommitLogEntry) -> bool {
        if !matches!(self.kind, PendingKind::Unmarked) || !self.accepts_repository(entry) {
            return false;
        }
        let first = &self.entries[0];
        first.message == entry.message
            && first.author.name == entry.author.name
            && first.author.email == entry.author.email
            && within_window(
                self.entries
                    .iter()
                    .map(|sibling| sibling.committer.time.seconds),
                entry.committer.time.seconds,
            )
            && within_window(
                self.entries
                    .iter()
                    .map(|sibling| sibling.author.time.seconds),
                entry.author.time.seconds,
            )
    }

    fn finish(self) -> CommitLogGroup {
        let provenance = match self.kind {
            PendingKind::Marker(marker) => CommitLogProvenance::Marker(marker),
            PendingKind::Unmarked if self.entries.len() > 1 => CommitLogProvenance::Heuristic,
            PendingKind::Unmarked => CommitLogProvenance::None,
            PendingKind::Opaque => CommitLogProvenance::MarkerInvalid,
        };
        finished_group(self.entries, provenance)
    }
}

fn within_window(times: impl Iterator<Item = i64>, candidate: i64) -> bool {
    let (mut oldest, mut newest) = (candidate, candidate);
    for time in times {
        oldest = oldest.min(time);
        newest = newest.max(time);
    }
    newest.abs_diff(oldest) <= HEURISTIC_WINDOW_SECONDS
}

fn finished_group(entries: Vec<CommitLogEntry>, provenance: CommitLogProvenance) -> CommitLogGroup {
    let ordering_timestamp_seconds = entries
        .iter()
        .map(|entry| entry.committer.time.seconds)
        .max()
        .expect("groups are constructed from at least one entry");
    CommitLogGroup {
        entries,
        provenance,
        ordering_timestamp_seconds,
    }
}

enum MarkerIdentity {
    Unmarked,
    Key(String),
    Unusable,
}

fn marker_identity(message: &[u8]) -> MarkerIdentity {
    let mut claims = message
        .split(|byte| *byte == b'\n')
        .filter(|line| marker_shaped_claim(line));
    let Some(claim) = claims.next() else {
        return MarkerIdentity::Unmarked;
    };
    if claims.next().is_some() {
        return MarkerIdentity::Unusable;
    }

    let Some(boundary) = message.windows(2).rposition(|bytes| bytes == b"\n\n") else {
        return MarkerIdentity::Unusable;
    };
    let block = message[boundary + 2..]
        .strip_suffix(b"\n")
        .unwrap_or(&message[boundary + 2..]);
    let mut lines = block.split(|byte| *byte == b'\n');
    let (Some(commit_line), Some(workspace_line)) = (lines.next(), lines.next()) else {
        return MarkerIdentity::Unusable;
    };
    if commit_line != claim {
        return MarkerIdentity::Unusable;
    }

    let Some(marker) = commit_line.strip_prefix(GWZ_COMMIT_ID_LINE) else {
        return MarkerIdentity::Unusable;
    };
    let Some(workspace_id) = workspace_line.strip_prefix(GWZ_WORKSPACE_ID_LINE) else {
        return MarkerIdentity::Unusable;
    };
    if !canonical_uuid_v7(marker) || workspace_id.is_empty() {
        return MarkerIdentity::Unusable;
    }

    match (lines.next(), lines.next()) {
        (None, None) => {}
        (Some(origin_line), None) if valid_origin_url_hash(origin_line) => {}
        _ => return MarkerIdentity::Unusable,
    }

    MarkerIdentity::Key(
        std::str::from_utf8(marker)
            .expect("a canonical UUID contains only ASCII")
            .to_owned(),
    )
}

fn marker_shaped_claim(line: &[u8]) -> bool {
    line.get(..GWZ_COMMIT_ID.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(GWZ_COMMIT_ID))
        && line
            .get(GWZ_COMMIT_ID.len())
            .is_none_or(|byte| matches!(byte, b':' | b'=' | b' ' | b'\t'))
}

fn canonical_uuid_v7(value: &[u8]) -> bool {
    value.len() == 36
        && value[8] == b'-'
        && value[13] == b'-'
        && value[14] == b'7'
        && value[18] == b'-'
        && matches!(value[19], b'8' | b'9' | b'a' | b'b')
        && value[23] == b'-'
        && value.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index)
                || byte.is_ascii_digit()
                || (b'a'..=b'f').contains(byte)
        })
}

fn valid_origin_url_hash(line: &[u8]) -> bool {
    line.strip_prefix(GWZ_ORIGIN_URL_HASH_LINE)
        .is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        })
}
