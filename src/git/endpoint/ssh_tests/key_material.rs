//! Key files for the SSH suites, on every platform: `ssh-keygen` (beside the fixture's `sshd` on Windows) makes them in
//! the test's temporary directory. The agent fixture that serves them is `key_fixture`, which only Unix has yet.
use crate::git::endpoint::{fixture_host, ssh_fixture as common};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// A new key of `ssh-keygen -t kind` (and `-b bits`) at `dir/name`, with no passphrase; its public key is
/// `dir/name.pub`.
pub(crate) fn keygen(dir: &Path, name: &str, kind: &str, bits: Option<u32>) -> PathBuf {
    let path = dir.join(name);
    let mut command = Command::new(fixture_host::programs().keygen);
    command.args(["-q", "-t", kind, "-N", "", "-f"]).arg(&path);
    if let Some(bits) = bits {
        command.args(["-b", &bits.to_string()]);
    }
    common::run(&mut command);
    path
}

/// The type and base64 blob of the public key `dir/name.pub`.
pub(crate) fn public(dir: &Path, name: &str) -> (String, String) {
    let text = fs::read_to_string(dir.join(format!("{name}.pub"))).unwrap();
    let mut fields = text.split_whitespace();
    (
        fields.next().unwrap().to_owned(),
        fields.next().unwrap().to_owned(),
    )
}
