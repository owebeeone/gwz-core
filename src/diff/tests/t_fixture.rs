//! The fixture's own promise: every `TempDir` is a directory that call created,
//! never one another `TempDir` already holds.
//!
//! On 2026-10-02 two `t_rename` tests that share a prefix got one directory:
//! the name came from the process id and a clock that macOS reads to the
//! microsecond, and `create_dir_all` accepted the existing directory. One test
//! then failed on `config.lock` and the other found no repository.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Barrier;

use super::TempDir;

const THREADS: usize = 8;
const PER_THREAD: usize = 200;

/// Parallel tests with one prefix, modelled as threads that start together and
/// each create directories in a tight loop. Every directory stays alive until
/// the end, as a running test's would, so a shared one cannot hide.
#[test]
fn temp_dirs_with_one_prefix_are_distinct_and_created_fresh() {
    let start = Barrier::new(THREADS);
    let held: Vec<(Vec<TempDir>, Vec<PathBuf>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|thread| {
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    let mut dirs = Vec::with_capacity(PER_THREAD);
                    let mut not_fresh = Vec::new();
                    for index in 0..PER_THREAD {
                        let dir = TempDir::new("fixture-unique");
                        // Fresh: nothing another TempDir wrote is visible here.
                        if fs::read_dir(dir.path()).unwrap().next().is_some() {
                            not_fresh.push(dir.path().to_path_buf());
                        }
                        fs::write(dir.path().join(format!("owner-{thread}-{index}")), "").unwrap();
                        dirs.push(dir);
                    }
                    (dirs, not_fresh)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect()
    });

    let not_fresh: Vec<&PathBuf> = held.iter().flat_map(|(_, paths)| paths).collect();
    assert!(
        not_fresh.is_empty(),
        "{} directories already held another TempDir's files, first {:?}",
        not_fresh.len(),
        not_fresh.first()
    );
    let mut seen = HashSet::new();
    let shared: Vec<&PathBuf> = held
        .iter()
        .flat_map(|(dirs, _)| dirs)
        .map(|dir| &dir.path)
        .filter(|path| !seen.insert(*path))
        .collect();
    assert!(
        shared.is_empty(),
        "{} of {} TempDirs reused a path, first {:?}",
        shared.len(),
        THREADS * PER_THREAD,
        shared.first()
    );
}
