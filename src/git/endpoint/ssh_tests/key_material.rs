//! Key files for the SSH suites, on every platform: `ssh-keygen` (beside the fixture's `sshd` on Windows) makes them in
//! the test's temporary directory. The agent fixture that serves them is `key_fixture` on Unix and `key_fixture_pipe` on
//! Windows.
use crate::git::endpoint::{fixture_host, ssh_fixture as common};
use base64::{Engine as _, engine::general_purpose::STANDARD};
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

/// Has the CA key `ca` certify `key` for `user` as `dir/name.pub`, which
/// `ssh-keygen` signs as `dir/name-cert.pub`; returns the certificate's type
/// and base64 blob.
pub(crate) fn certify(
    dir: &Path,
    ca: &Path,
    name: &str,
    key: &(String, String),
    user: &str,
) -> (String, String) {
    let path = dir.join(format!("{name}.pub"));
    fs::write(&path, format!("{} {}\n", key.0, key.1)).unwrap();
    common::run(
        Command::new(fixture_host::programs().keygen)
            .args(["-q", "-s"])
            .arg(ca)
            .args(["-I", "tr2-8", "-n", user])
            .arg(&path),
    );
    public(dir, &format!("{name}-cert"))
}

/// The security key whose point is the plain Ed25519 or ECDSA P-256 key
/// `blob`'s, with the application `ssh:`, as its type and base64 blob.
pub(crate) fn security_key(blob: &str) -> (String, String) {
    let raw = STANDARD.decode(blob).unwrap();
    let mut fields = Vec::new();
    let mut input = &raw[..];
    while !input.is_empty() {
        let length = u32::from_be_bytes(input[..4].try_into().unwrap()) as usize;
        fields.push(&input[4..4 + length]);
        input = &input[4 + length..];
    }
    let kind = match fields[0] {
        b"ssh-ed25519" => "sk-ssh-ed25519@openssh.com",
        b"ecdsa-sha2-nistp256" => "sk-ecdsa-sha2-nistp256@openssh.com",
        other => panic!("no security key of {}", String::from_utf8_lossy(other)),
    };
    let mut out = Vec::new();
    let tail: [&[u8]; 1] = [b"ssh:"];
    for field in [kind.as_bytes()]
        .into_iter()
        .chain(fields[1..].iter().copied())
        .chain(tail)
    {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field);
    }
    (kind.to_owned(), STANDARD.encode(out))
}
