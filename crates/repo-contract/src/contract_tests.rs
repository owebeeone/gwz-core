//! Conformance suites and contract-faithful fakes for the repository ports.
//!
//! Enabled by the `contract-tests` feature (dev-dependencies only). Pure
//! consumers (`gwz-work-detector`, `gwz-history-check`) test against
//! [`InMemoryObjectReader`] graphs and plain [`WorkObservation`] values;
//! the real inspector (`gwz-repo-inspect`) runs [`object_reader_conformance`]
//! and [`inspector_conformance`] against small real repositories built by the
//! dev-only fixture crate.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{
    HeadState, LayoutError, ObjectFormat, ObjectId, ObjectKind, ObjectReader, ObjectRecord,
    Observation, ProtectedRoot, ProtectedRoots, ReadError, ReadLimits, RepoInspector,
    RepositoryInfo, RootSource, RootSourceKind, WorkObservation,
};

/// Deterministic object ids for fixtures: `oid(format, n)` is the digest whose
/// every byte is `n`.
pub fn oid(format: ObjectFormat, byte: u8) -> ObjectId {
    ObjectId::from_bytes(format, &vec![byte; format.digest_len()]).expect("fixture id")
}

/// An in-memory object graph. Faithful to the contract: missing objects are
/// [`ReadError::Missing`], oversized objects are [`ReadError::LimitExceeded`],
/// reads are repeatable and never mutate the graph.
#[derive(Clone, Debug, Default)]
pub struct InMemoryObjectReader {
    objects: BTreeMap<ObjectId, ObjectRecord>,
    roots: ProtectedRoots,
    reads: RefCell<Vec<ObjectId>>,
}

impl InMemoryObjectReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, record: ObjectRecord) -> &mut Self {
        self.objects.insert(record.oid.clone(), record);
        self
    }

    pub fn commit(&mut self, oid: ObjectId, tree: ObjectId, parents: Vec<ObjectId>) -> &mut Self {
        let mut edges = vec![tree];
        edges.extend(parents);
        self.insert(ObjectRecord {
            oid,
            kind: ObjectKind::Commit,
            size: 200,
            edges,
        })
    }

    pub fn tree(&mut self, oid: ObjectId, entries: Vec<ObjectId>) -> &mut Self {
        self.insert(ObjectRecord {
            oid,
            kind: ObjectKind::Tree,
            size: 40 * (entries.len() as u64 + 1),
            edges: entries,
        })
    }

    pub fn blob(&mut self, oid: ObjectId, size: u64) -> &mut Self {
        self.insert(ObjectRecord {
            oid,
            kind: ObjectKind::Blob,
            size,
            edges: Vec::new(),
        })
    }

    pub fn tag(&mut self, oid: ObjectId, target: ObjectId) -> &mut Self {
        self.insert(ObjectRecord {
            oid,
            kind: ObjectKind::Tag,
            size: 120,
            edges: vec![target],
        })
    }

    pub fn root(&mut self, source: RootSource, oid: ObjectId) -> &mut Self {
        self.roots.roots.push(ProtectedRoot { source, oid });
        self
    }

    /// Every id read so far, in order (for call-count assertions).
    pub fn reads(&self) -> Vec<ObjectId> {
        self.reads.borrow().clone()
    }
}

impl ObjectReader for InMemoryObjectReader {
    fn retained_roots(&self) -> Result<ProtectedRoots, ReadError> {
        Ok(self.roots.clone())
    }

    fn read_object(&self, oid: &ObjectId, limits: &ReadLimits) -> Result<ObjectRecord, ReadError> {
        self.reads.borrow_mut().push(oid.clone());
        let record = self
            .objects
            .get(oid)
            .ok_or_else(|| ReadError::Missing { oid: oid.clone() })?;
        if record.size > limits.max_object_bytes {
            return Err(ReadError::LimitExceeded {
                oid: oid.clone(),
                size: record.size,
                limit: limits.max_object_bytes,
            });
        }
        Ok(record.clone())
    }
}

/// What an [`ObjectReader`] under test is expected to contain.
#[derive(Clone, Debug)]
pub struct GraphFixture {
    /// Every object the reader must serve, with exact kinds and edges.
    pub objects: Vec<ObjectRecord>,
    /// The roots `retained_roots` must report.
    pub roots: ProtectedRoots,
    /// An id that must not be present.
    pub missing: ObjectId,
}

impl GraphFixture {
    /// A three-object fixture (commit -> tree -> blob) with one `HEAD` root,
    /// plus its in-memory reader; the real inspector builds the same shape in
    /// a real repository and passes its own reader.
    pub fn small(format: ObjectFormat) -> (Self, InMemoryObjectReader) {
        let blob = oid(format, 0x01);
        let tree = oid(format, 0x02);
        let commit = oid(format, 0x03);
        let mut reader = InMemoryObjectReader::new();
        reader
            .blob(blob.clone(), 5)
            .tree(tree.clone(), vec![blob.clone()])
            .commit(commit.clone(), tree.clone(), Vec::new())
            .root(RootSource::Head, commit.clone());
        let fixture = Self {
            objects: vec![
                reader.objects[&blob].clone(),
                reader.objects[&tree].clone(),
                reader.objects[&commit].clone(),
            ],
            roots: reader.roots.clone(),
            missing: oid(format, 0xEE),
        };
        (fixture, reader)
    }

    /// [`Self::small`] plus an annotated tag object at the commit, whose ref
    /// `refs/tags/v1` is reported **once**, as `RootSource::AnnotatedTag` at
    /// the tag object's id (LCM1.0c follow-up 3, T-1). A reader that reports
    /// the same tag as `Ref { "refs/tags/v1" }` -- or as both -- fails
    /// [`reports_exactly_the_fixture_roots`] against this fixture; the real
    /// inspector builds the same shape in a real repository.
    pub fn tagged(format: ObjectFormat) -> (Self, InMemoryObjectReader) {
        let (mut fixture, mut reader) = Self::small(format);
        let commit = fixture.objects[2].oid.clone();
        let tag = oid(format, 0x04);
        reader.tag(tag.clone(), commit).root(
            RootSource::AnnotatedTag {
                name: "refs/tags/v1".to_owned(),
            },
            tag.clone(),
        );
        fixture.objects.push(reader.objects[&tag].clone());
        fixture.roots = reader.roots.clone();
        (fixture, reader)
    }
}

/// Run the [`ObjectReader`] conformance cases against `reader`, with exact
/// root equality.
pub fn object_reader_conformance<R: ObjectReader>(reader: &R, fixture: &GraphFixture) {
    object_reader_conformance_allowing(reader, fixture, &[]);
}

/// Run the conformance cases against `reader`, allowing it to report roots
/// beyond the fixture's whose kind is in `allowed` (lane T proposal T-2,
/// LCM1.0c follow-up 3): a real reader also reports retained reflog entries
/// and stash entries that a fixture cannot enumerate portably. Every fixture
/// root must still be reported exactly, `unknown` must be empty, and an extra
/// root of any other kind fails. With `allowed` empty this is
/// [`object_reader_conformance`]'s exact equality.
pub fn object_reader_conformance_allowing<R: ObjectReader>(
    reader: &R,
    fixture: &GraphFixture,
    allowed: &[RootSourceKind],
) {
    reports_the_fixture_roots_allowing(reader, fixture, allowed);
    serves_every_fixture_object_with_its_edges(reader, fixture);
    refuses_a_missing_object_typed(reader, fixture);
    refuses_an_oversized_object_within_limits(reader, fixture);
    reads_are_repeatable(reader, fixture);
}

pub fn reports_exactly_the_fixture_roots<R: ObjectReader>(reader: &R, fixture: &GraphFixture) {
    let roots = reader.retained_roots().expect("roots are readable");
    assert_eq!(roots, fixture.roots);
}

/// [`reports_exactly_the_fixture_roots`] when `allowed` is empty; otherwise
/// the fixture's roots are a subset of what is reported and every extra root
/// is of an allowed kind (T-2).
pub fn reports_the_fixture_roots_allowing<R: ObjectReader>(
    reader: &R,
    fixture: &GraphFixture,
    allowed: &[RootSourceKind],
) {
    if allowed.is_empty() {
        return reports_exactly_the_fixture_roots(reader, fixture);
    }
    let reported = reader.retained_roots().expect("roots are readable");
    assert!(
        reported.unknown.is_empty(),
        "the reader established every root: {:?}",
        reported.unknown
    );
    for expected in &fixture.roots.roots {
        assert!(
            reported.roots.contains(expected),
            "fixture root {expected:?} was not reported: {:?}",
            reported.roots
        );
    }
    for root in &reported.roots {
        if !fixture.roots.roots.contains(root) {
            assert!(
                allowed.contains(&root.source.kind()),
                "extra root {root:?} is not of an allowed kind {allowed:?}"
            );
        }
    }
}

pub fn serves_every_fixture_object_with_its_edges<R: ObjectReader>(
    reader: &R,
    fixture: &GraphFixture,
) {
    for expected in &fixture.objects {
        let record = reader
            .read_object(&expected.oid, &ReadLimits::default())
            .unwrap_or_else(|error| panic!("{}: {error}", expected.oid));
        assert_eq!(&record, expected);
    }
}

pub fn refuses_a_missing_object_typed<R: ObjectReader>(reader: &R, fixture: &GraphFixture) {
    let error = reader
        .read_object(&fixture.missing, &ReadLimits::default())
        .expect_err("missing object refuses");
    assert_eq!(
        error,
        ReadError::Missing {
            oid: fixture.missing.clone()
        }
    );
}

pub fn refuses_an_oversized_object_within_limits<R: ObjectReader>(
    reader: &R,
    fixture: &GraphFixture,
) {
    let largest = fixture
        .objects
        .iter()
        .max_by_key(|record| record.size)
        .expect("fixture has objects");
    assert!(largest.size > 0, "fixture objects have a size");
    let limits = ReadLimits::new(largest.size - 1);
    match reader.read_object(&largest.oid, &limits) {
        Err(ReadError::LimitExceeded { oid, size, limit }) => {
            assert_eq!(oid, largest.oid);
            assert_eq!(size, largest.size);
            assert_eq!(limit, largest.size - 1);
        }
        other => panic!("oversized read must be LimitExceeded, got {other:?}"),
    }
}

pub fn reads_are_repeatable<R: ObjectReader>(reader: &R, fixture: &GraphFixture) {
    let first = reader.read_object(&fixture.objects[0].oid, &ReadLimits::default());
    let second = reader.read_object(&fixture.objects[0].oid, &ReadLimits::default());
    assert_eq!(first, second);
    assert_eq!(
        reader.retained_roots().unwrap(),
        reader.retained_roots().unwrap()
    );
}

/// A scripted inspector for orchestration tests: layouts, work and history
/// are answered from what the test registered per path; an unregistered path
/// is `NotARepository`, and observations default to unknown-unimplemented
/// rather than to a clean report.
#[derive(Debug, Default)]
pub struct ScriptedRepoInspector {
    layouts: BTreeMap<PathBuf, Result<RepositoryInfo, LayoutError>>,
    work: BTreeMap<PathBuf, Observation<WorkObservation>>,
    history: BTreeMap<PathBuf, Observation<ProtectedRoots>>,
    calls: RefCell<Vec<String>>,
}

impl ScriptedRepoInspector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn layout(
        &mut self,
        path: impl Into<PathBuf>,
        result: Result<RepositoryInfo, LayoutError>,
    ) {
        self.layouts.insert(path.into(), result);
    }

    pub fn work(&mut self, path: impl Into<PathBuf>, observation: Observation<WorkObservation>) {
        self.work.insert(path.into(), observation);
    }

    pub fn history(&mut self, path: impl Into<PathBuf>, observation: Observation<ProtectedRoots>) {
        self.history.insert(path.into(), observation);
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl RepoInspector for ScriptedRepoInspector {
    fn inspect_layout(&self, path: &Path) -> Result<RepositoryInfo, LayoutError> {
        self.calls
            .borrow_mut()
            .push(format!("inspect_layout {}", path.display()));
        self.layouts.get(path).cloned().unwrap_or_else(|| {
            Err(LayoutError::NotARepository {
                path: path.to_path_buf(),
            })
        })
    }

    fn observe_work(&self, repository: &RepositoryInfo) -> Observation<WorkObservation> {
        self.calls
            .borrow_mut()
            .push(format!("observe_work {}", repository.path.display()));
        self.work
            .get(&repository.path)
            .cloned()
            .unwrap_or_else(|| Observation::unimplemented("observe_work"))
    }

    fn inventory_history(&self, repository: &RepositoryInfo) -> Observation<ProtectedRoots> {
        self.calls
            .borrow_mut()
            .push(format!("inventory_history {}", repository.path.display()));
        self.history
            .get(&repository.path)
            .cloned()
            .unwrap_or_else(|| Observation::unimplemented("inventory_history"))
    }
}

/// [`RepoInspector`] conformance: the invariants any inspector can be held
/// to without a fixture: a non-repository path refuses typed, and a known
/// layout observed twice yields the same answer.
pub fn inspector_conformance<I: RepoInspector>(
    inspector: &I,
    not_a_repository: &Path,
    repository: Option<&Path>,
) {
    non_repository_path_refuses_typed(inspector, not_a_repository);
    if let Some(repository) = repository {
        observations_are_repeatable(inspector, repository);
    }
}

pub fn non_repository_path_refuses_typed<I: RepoInspector>(inspector: &I, path: &Path) {
    match inspector.inspect_layout(path) {
        Err(LayoutError::NotARepository { path: reported }) => assert_eq!(reported, path),
        Err(LayoutError::Unimplemented { .. }) => {}
        other => panic!("non-repository must refuse typed, got {other:?}"),
    }
}

pub fn observations_are_repeatable<I: RepoInspector>(inspector: &I, repository: &Path) {
    let info = inspector
        .inspect_layout(repository)
        .expect("fixture repository is admitted");
    assert_eq!(inspector.inspect_layout(repository).unwrap(), info);
    // I-1 (LCM1.0c follow-up 3): the layout's paths are resolved, and an
    // attached or unborn HEAD names its branch by full reference name.
    for (label, path) in [
        ("path", &info.path),
        ("git_dir", &info.git_dir),
        ("common_dir", &info.common_dir),
    ] {
        assert!(
            path.is_absolute(),
            "RepositoryInfo::{label} must be a resolved path: {}",
            path.display()
        );
    }
    match &info.head {
        HeadState::Attached { branch, .. } | HeadState::Unborn { branch } => assert!(
            branch.starts_with("refs/"),
            "HeadState branch must be a full reference name: {branch}"
        ),
        HeadState::Detached { .. } => {}
    }
    assert_eq!(inspector.observe_work(&info), inspector.observe_work(&info));
    assert_eq!(
        inspector.inventory_history(&info),
        inspector.inventory_history(&info)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HeadState, UnknownKind};

    #[test]
    fn in_memory_reader_satisfies_the_conformance_suite_for_both_formats() {
        for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
            let (fixture, reader) = GraphFixture::small(format);
            object_reader_conformance(&reader, &fixture);
            assert!(!reader.reads().is_empty(), "reads are recorded");
        }
    }

    #[test]
    fn scripted_inspector_defaults_to_refusal_and_unknown_never_to_clean() {
        let inspector = ScriptedRepoInspector::new();
        inspector_conformance(&inspector, Path::new("/nowhere"), None);
        let info = RepositoryInfo {
            path: PathBuf::from("/repo"),
            git_dir: PathBuf::from("/repo/.git"),
            common_dir: PathBuf::from("/repo/.git"),
            bare: false,
            object_format: ObjectFormat::Sha1,
            head: HeadState::Unborn {
                branch: "main".to_owned(),
            },
        };
        let Observation::Unknown(reasons) = inspector.observe_work(&info) else {
            panic!("unscripted work observation must be unknown");
        };
        assert_eq!(reasons[0].kind, UnknownKind::Unimplemented);
        assert!(inspector.inventory_history(&info).is_unknown());
        assert_eq!(inspector.calls().len(), 3);
    }

    #[test]
    fn scripted_layouts_are_repeatable() {
        let mut inspector = ScriptedRepoInspector::new();
        // An absolute path on every host: the I-1 assertions in the suite
        // require a resolved layout, and `/repo` is relative on Windows.
        let repo = std::env::temp_dir().join("scripted-repo");
        let info = RepositoryInfo {
            path: repo.clone(),
            git_dir: repo.join(".git"),
            common_dir: repo.join(".git"),
            bare: false,
            object_format: ObjectFormat::Sha256,
            head: HeadState::Attached {
                branch: "refs/heads/main".to_owned(),
                target: oid(ObjectFormat::Sha256, 9),
            },
        };
        inspector.layout(repo.clone(), Ok(info.clone()));
        inspector.work(repo.clone(), Observation::Known(WorkObservation::default()));
        inspector.history(repo.clone(), Observation::Known(ProtectedRoots::default()));
        inspector_conformance(&inspector, Path::new("/nowhere"), Some(&repo));
    }

    /// T-1 (LCM1.0c follow-up 3): the tagged fixture carries the annotated
    /// tag once, as `AnnotatedTag` at the tag object, and the in-memory
    /// reader conforms to it exactly.
    #[test]
    fn the_tagged_fixture_reports_the_annotated_tag_once_at_the_tag_object() {
        for format in [ObjectFormat::Sha1, ObjectFormat::Sha256] {
            let (fixture, reader) = GraphFixture::tagged(format);
            object_reader_conformance(&reader, &fixture);
            let tags: Vec<&ProtectedRoot> = fixture
                .roots
                .roots
                .iter()
                .filter(|root| root.source.kind() == RootSourceKind::AnnotatedTag)
                .collect();
            assert_eq!(tags.len(), 1, "{:?}", fixture.roots);
            let record = reader
                .read_object(&tags[0].oid, &ReadLimits::default())
                .unwrap();
            assert_eq!(record.kind, ObjectKind::Tag, "the root is the tag object");
            assert_eq!(record.edges, vec![fixture.objects[2].oid.clone()]);
            assert!(
                !fixture.roots.roots.iter().any(|root| matches!(
                    &root.source,
                    RootSource::Ref { name } if name == "refs/tags/v1"
                )),
                "the tag is never also a Ref: {:?}",
                fixture.roots
            );
        }
    }

    /// T-1: a reader spelling the annotated tag as `Ref` fails the exact
    /// comparison; so would one reporting it under both sources.
    #[test]
    #[should_panic(expected = "assertion `left == right` failed")]
    fn a_reader_spelling_the_annotated_tag_as_a_ref_fails_exact_equality() {
        let (fixture, _) = GraphFixture::tagged(ObjectFormat::Sha1);
        let mut reader = InMemoryObjectReader::new();
        for record in &fixture.objects {
            reader.insert(record.clone());
        }
        for root in &fixture.roots.roots {
            let source = match &root.source {
                RootSource::AnnotatedTag { name } => RootSource::Ref { name: name.clone() },
                other => other.clone(),
            };
            reader.root(source, root.oid.clone());
        }
        reports_exactly_the_fixture_roots(&reader, &fixture);
    }

    /// T-2 (LCM1.0c follow-up 3): an extra root of an allowed kind passes the
    /// allowing comparison while the exact one stays exact.
    #[test]
    fn an_extra_root_of_an_allowed_kind_passes_the_allowing_comparison() {
        let (fixture, mut reader) = GraphFixture::small(ObjectFormat::Sha1);
        let commit = fixture.objects[2].oid.clone();
        reader.root(
            RootSource::Reflog {
                reference: "HEAD".to_owned(),
                index: 3,
            },
            commit,
        );
        object_reader_conformance_allowing(&reader, &fixture, &[RootSourceKind::Reflog]);
        assert_ne!(
            reader.retained_roots().unwrap(),
            fixture.roots,
            "the allowance is what makes this pass; exact equality would not"
        );
    }

    #[test]
    #[should_panic(expected = "is not of an allowed kind")]
    fn an_extra_root_of_another_kind_fails_the_allowing_comparison() {
        let (fixture, mut reader) = GraphFixture::small(ObjectFormat::Sha1);
        let commit = fixture.objects[2].oid.clone();
        reader.root(
            RootSource::Reflog {
                reference: "HEAD".to_owned(),
                index: 3,
            },
            commit,
        );
        reports_the_fixture_roots_allowing(&reader, &fixture, &[RootSourceKind::Stash]);
    }
}
