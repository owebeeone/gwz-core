//! The fixture's host key file, as the server presents it: OpenSSH's own private key form where the fixture's libssh2
//! reads it, and a traditional PEM RSA key where it does not (`fixture_host::fixture_key_arguments`).
use russh::keys::{PrivateKey, decode_secret_key};
use std::{fs, io, path::Path};

pub(super) fn load(path: &Path) -> io::Result<PrivateKey> {
    let text = fs::read_to_string(path)?;
    decode_secret_key(&text, None).map_err(io::Error::other)
}
