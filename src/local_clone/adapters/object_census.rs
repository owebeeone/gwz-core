//! The object census behind design §4.0 dest-complete's bound (LCM1.1 fix
//! 2, lane C, 2026-09-06): how many objects the destination's own store
//! holds, counted from the copied files before the connectivity walk, so
//! the walk's ceiling is what was actually copied rather than a constant.
//!
//! The count is cheap -- one directory listing per loose fan-out directory
//! and one 4-byte read per pack index -- and it is an upper bound on what a
//! walk over that store can visit: every object the walk reads is one of
//! these files, and an object present both loose and packed is counted
//! twice, never zero times. The walk itself is `gwz-history-check`'s
//! (`check_connectivity`); this module only measures the store and derives
//! the [`Limits`] handed to it.
//!
//! Measured cost of the walk (Darwin 25.6 arm64, Apple M-series, warm; the
//! LCM1.1 checkpoint §14 carries the table): 12 µs per object on gwz-core's
//! own store (9,242 objects, 7,166 of them packed: 105 ms), 30 µs on
//! gwz-cli's (1,921: 55 ms), 65-73 µs on the gwz-dev root's all-loose store
//! (4,957: 250-280 ms) and 5.6 µs on taut's (1,530: 8 ms); on a synthetic
//! 80 k-object linear history, 61 µs packed and 112 µs loose (4.9 s and
//! 9.0 s). So roughly one second per 10-80 k objects, and the census itself
//! costs 1-24 ms. The walk charged 117 bytes of bookkeeping per distinct
//! object on gwz-core and 186 at peak on the synthetic history, so the
//! library's default 256 MiB cap is the outer ceiling: about 1.4 million
//! objects per repository, past which dest-complete refuses typed
//! (`destination_incomplete`) rather than walking on.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use gwz_history_check::Limits;

/// Bookkeeping bytes allowed per counted object, with headroom over the
/// measured peak of about 186 bytes per distinct object.
pub const BOOKKEEPING_BYTES_PER_OBJECT: u64 = 512;

/// A fixed allowance for the roots and the walk's stack, on top of the
/// per-object budget.
pub const BOOKKEEPING_SLACK_BYTES: u64 = 4 * 1024 * 1024;

/// The magic that opens a version-2 pack index; a version-1 index has no
/// header and its fan-out table starts at offset 0.
const PACK_INDEX_V2_MAGIC: [u8; 4] = [0xff, 0x74, 0x4f, 0x63];

/// What one object store holds, by file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectCensus {
    /// Loose objects: files named by a hex id under `objects/<xx>/`.
    pub loose: u64,
    /// Packed objects: the sum of every pack index's fan-out total.
    pub packed: u64,
    /// Pack indexes read.
    pub packs: u64,
}

impl ObjectCensus {
    /// The ceiling on distinct objects a walk over this store can visit.
    pub fn total(&self) -> u64 {
        self.loose.saturating_add(self.packed)
    }
}

/// Count the objects in the store under `common_dir` (`<common dir>/objects`).
/// Alternates are not consulted: dest-complete admits a store only with
/// alternates absent, and this census is of the store itself.
pub fn census_of(common_dir: &Path) -> Result<ObjectCensus, String> {
    let objects = common_dir.join("objects");
    let mut census = ObjectCensus::default();
    let entries = fs::read_dir(&objects).map_err(|error| describe(&objects, &error))?;
    for entry in entries {
        let entry = entry.map_err(|error| describe(&objects, &error))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let file_type = entry
            .file_type()
            .map_err(|error| describe(&entry.path(), &error))?;
        if name.len() == 2 && name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            if !file_type.is_dir() {
                continue;
            }
            let fanout = entry.path();
            for object in fs::read_dir(&fanout).map_err(|error| describe(&fanout, &error))? {
                let object = object.map_err(|error| describe(&fanout, &error))?;
                let is_object = object.file_name().to_str().is_some_and(|name| {
                    (name.len() == 38 || name.len() == 62)
                        && name.bytes().all(|byte| byte.is_ascii_hexdigit())
                }) && object
                    .file_type()
                    .map_err(|error| describe(&object.path(), &error))?
                    .is_file();
                if is_object {
                    census.loose += 1;
                }
            }
        } else if name == "pack" && file_type.is_dir() {
            let packs = entry.path();
            let mut indexes: Vec<_> = fs::read_dir(&packs)
                .map_err(|error| describe(&packs, &error))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| describe(&packs, &error))?
                .into_iter()
                .filter(|index| index.path().extension().is_some_and(|ext| ext == "idx"))
                .map(|index| index.path())
                .collect();
            indexes.sort();
            for index in indexes {
                census.packed = census.packed.saturating_add(pack_index_total(&index)?);
                census.packs += 1;
            }
        }
    }
    Ok(census)
}

/// The number of objects a pack index describes: the last entry of its
/// 256-entry fan-out table (offset 8 in a version-2 index, 0 in version 1).
fn pack_index_total(index: &Path) -> Result<u64, String> {
    let mut file = fs::File::open(index).map_err(|error| describe(index, &error))?;
    let mut header = [0u8; 8];
    file.read_exact(&mut header)
        .map_err(|error| describe(index, &error))?;
    let fanout_offset: u64 = if header[..4] == PACK_INDEX_V2_MAGIC {
        let version = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
        if version != 2 {
            return Err(format!(
                "{}: pack index version {version} is not one this census reads",
                index.display()
            ));
        }
        8
    } else {
        0
    };
    file.seek(SeekFrom::Start(fanout_offset + 255 * 4))
        .map_err(|error| describe(index, &error))?;
    let mut total = [0u8; 4];
    file.read_exact(&mut total)
        .map_err(|error| describe(index, &error))?;
    Ok(u64::from(u32::from_be_bytes(total)))
}

fn describe(path: &Path, error: &io::Error) -> String {
    format!("{}: {error}", path.display())
}

/// The [`Limits`] for one destination repository's connectivity walk,
/// derived from what was copied: exactly the roots the inventory found (the
/// walk starts from every one of them and needs no cap of its own), and a
/// bookkeeping budget of [`BOOKKEEPING_BYTES_PER_OBJECT`] per counted object
/// plus [`BOOKKEEPING_SLACK_BYTES`], never above the library's default cap,
/// which stays the documented outer ceiling.
pub fn connectivity_limits(roots: usize, census: &ObjectCensus) -> Limits {
    let default = Limits::default();
    let derived = census
        .total()
        .saturating_mul(BOOKKEEPING_BYTES_PER_OBJECT)
        .saturating_add(BOOKKEEPING_SLACK_BYTES);
    Limits {
        max_roots: (roots as u64).max(1),
        max_bookkeeping_bytes: derived.min(default.max_bookkeeping_bytes),
        reads: default.reads,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::Command;

    use super::*;

    /// A version-2 pack index header and fan-out table describing `total`
    /// objects, and nothing else -- enough for the census, which reads no
    /// further.
    fn write_index(path: &Path, version_2: bool, total: u32) {
        let mut bytes = Vec::new();
        if version_2 {
            bytes.extend_from_slice(&PACK_INDEX_V2_MAGIC);
            bytes.extend_from_slice(&2u32.to_be_bytes());
        }
        for slot in 0..256u32 {
            let value = if slot == 255 { total } else { total / 2 };
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        fs::File::create(path).unwrap().write_all(&bytes).unwrap();
    }

    #[test]
    fn the_census_counts_loose_files_by_shape_and_packs_by_their_fanout() {
        let tree = gwz_local_testrepo::TempTree::new("census");
        let common = tree.dir("repo/.git");
        let objects = common.join("objects");
        // Two SHA-1 objects (2 + 38 hex) and one SHA-256 object (2 + 62).
        tree.file(&format!("repo/.git/objects/ab/{}", "c".repeat(38)), b"x");
        tree.file(&format!("repo/.git/objects/ab/{}", "d".repeat(38)), b"x");
        tree.file(
            &format!("repo/.git/objects/ff/{}", "e".repeat(62)),
            b"sha256",
        );
        // Not objects: a temp file git may leave, a name of the wrong shape,
        // and `info/` which holds no objects.
        tree.file("repo/.git/objects/ab/tmp_obj_Q1w2e3", b"x");
        tree.file("repo/.git/objects/ab/notanobject", b"x");
        tree.file("repo/.git/objects/info/packs", b"");
        tree.dir("repo/.git/objects/pack");
        write_index(&objects.join("pack/pack-1.idx"), true, 40);
        write_index(&objects.join("pack/pack-2.idx"), false, 2);
        tree.file("repo/.git/objects/pack/pack-1.pack", b"PACK");

        let census = census_of(&common).unwrap();
        assert_eq!(
            census,
            ObjectCensus {
                loose: 3,
                packed: 42,
                packs: 2,
            }
        );
        assert_eq!(census.total(), 45);
    }

    #[test]
    fn the_census_agrees_with_git_before_and_after_a_repack() {
        let tree = gwz_local_testrepo::TempTree::new("census-git");
        let repo = tree.repo("repo");
        repo.commit_files("one", &[("a.txt", b"a\n"), ("b.txt", b"b\n")]);
        repo.commit_files("two", &[("a.txt", b"a2\n")]);
        // Two commits, two trees, three blobs: seven loose objects.
        let loose = census_of(&repo.common_dir()).unwrap();
        assert_eq!(
            loose,
            ObjectCensus {
                loose: 7,
                packed: 0,
                packs: 0,
            }
        );
        let status = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["repack", "-adq"])
            .status()
            .expect("git is available");
        assert!(status.success());
        let packed = census_of(&repo.common_dir()).unwrap();
        assert_eq!(
            packed,
            ObjectCensus {
                loose: 0,
                packed: 7,
                packs: 1,
            }
        );
    }

    #[test]
    fn the_census_refuses_a_store_it_cannot_read() {
        let tree = gwz_local_testrepo::TempTree::new("census-missing");
        let error = census_of(&tree.join("absent/.git")).unwrap_err();
        assert!(error.contains("objects"), "{error}");
    }

    #[test]
    fn the_limits_are_exact_roots_and_a_census_derived_budget_under_the_ceiling() {
        let default = Limits::default();
        let small = connectivity_limits(
            12,
            &ObjectCensus {
                loose: 100,
                packed: 900,
                packs: 1,
            },
        );
        assert_eq!(small.max_roots, 12);
        assert_eq!(
            small.max_bookkeeping_bytes,
            1000 * BOOKKEEPING_BYTES_PER_OBJECT + BOOKKEEPING_SLACK_BYTES
        );
        assert_eq!(small.reads, default.reads);
        // No roots is still a walkable request (the walk is a no-op).
        assert_eq!(
            connectivity_limits(0, &ObjectCensus::default()).max_roots,
            1
        );
        // A store past the library's ceiling keeps the ceiling: the walk
        // refuses typed there instead of growing its budget.
        let huge = connectivity_limits(
            1,
            &ObjectCensus {
                loose: 0,
                packed: 10_000_000,
                packs: 3,
            },
        );
        assert_eq!(huge.max_bookkeeping_bytes, default.max_bookkeeping_bytes);
    }
}
