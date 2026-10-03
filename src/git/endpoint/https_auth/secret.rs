//! Endpoint-local credential parsing and Basic construction.
use super::*;
use crate::session_host::environment::overwrite;

/// Every endpoint-owned allocation that contains credential bytes is wiped.
pub(super) struct SecretBuffer(pub(super) Vec<u8>);
impl Drop for SecretBuffer {
    fn drop(&mut self) {
        overwrite(&mut self.0);
    }
}

pub(crate) struct Secret {
    pub(super) username: Vec<u8>,
    pub(super) password: Vec<u8>,
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        pub(crate) struct SecretHeader(SecretBuffer, Option<std::sync::Arc<std::sync::Mutex<Vec<(usize, bool)>>>>);
    } else {
        pub(crate) struct SecretHeader(SecretBuffer);
    }
}
impl Drop for SecretHeader {
    fn drop(&mut self) {
        overwrite(&mut self.0.0);
        cfg_if::cfg_if! { if #[cfg(test)] {
            if let Some(records) = &self.1 {
                records.lock().unwrap().push((self.0.0.len(), self.0.0.iter().all(|byte| *byte == 0)));
            }
        } }
    }
}
impl SecretHeader {
    fn owned(buffer: SecretBuffer) -> Self {
        cfg_if::cfg_if! {
            if #[cfg(test)] { Self(buffer, None) }
            else { Self(buffer) }
        }
    }
    cfg_if::cfg_if! { if #[cfg(test)] {
        pub(crate) fn observe_wipe(&mut self, records: std::sync::Arc<std::sync::Mutex<Vec<(usize, bool)>>>) {
            self.1 = Some(records);
        }
    } }

    pub(crate) fn channel_binding_digest(digest: &mut Vec<u8>) -> Option<gwz_sspi::SecretBytes> {
        let binding = if matches!(digest.len(), 32 | 48 | 64) {
            const PREFIX: &[u8] = b"tls-server-end-point:";
            let mut owner = Self::owned(SecretBuffer(vec![0; PREFIX.len() + digest.len()]));
            owner.0.0[..PREFIX.len()].copy_from_slice(PREFIX);
            owner.0.0[PREFIX.len()..].copy_from_slice(digest);
            Some(gwz_sspi::SecretBytes::new(owner.as_bytes()))
        } else {
            None
        };
        overwrite(digest);
        binding
    }
    pub(crate) fn from_bytes(source: &[u8]) -> Self {
        let mut owner = Self::owned(SecretBuffer(vec![0; source.len()]));
        owner.0.0.copy_from_slice(source);
        owner
    }
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0.0
    }
}
impl std::ops::Deref for SecretHeader {
    type Target = str;
    fn deref(&self) -> &str {
        std::str::from_utf8(self.as_bytes()).expect("Basic is ASCII")
    }
}
impl fmt::Debug for SecretHeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretHeader(REDACTED)")
    }
}
impl PartialEq<&str> for SecretHeader {
    fn eq(&self, other: &&str) -> bool {
        &**self == *other
    }
}
impl Secret {
    pub(crate) fn has_native_identity(&self) -> bool {
        let username = self.username.strip_suffix(&[0]).unwrap_or(&self.username);
        let user = username
            .split(|byte| *byte == b'\\')
            .next_back()
            .unwrap_or(username);
        !user.is_empty()
    }
    pub(crate) fn native_identity(
        &self,
    ) -> Result<gwz_sspi::Identity, gwz_transport::protocol::ErrorCode> {
        use gwz_transport::protocol::ErrorCode;
        let username =
            std::str::from_utf8(self.username.strip_suffix(&[0]).unwrap_or(&self.username))
                .map_err(|_| ErrorCode::Authentication)?;
        let password =
            std::str::from_utf8(&self.password).map_err(|_| ErrorCode::Authentication)?;
        let (domain, user) = username.split_once('\\').unwrap_or(("", username));
        if user.is_empty() || user.len() > 8192 || domain.len() > 8192 || password.len() > 8192 {
            return Err(ErrorCode::Authentication);
        }
        Ok(gwz_sspi::Identity::Explicit {
            user: gwz_sspi::SecretText::new(user).map_err(|_| ErrorCode::Authentication)?,
            domain: gwz_sspi::SecretText::new(domain).map_err(|_| ErrorCode::Authentication)?,
            password: gwz_sspi::SecretText::new(password).map_err(|_| ErrorCode::Authentication)?,
        })
    }
    pub(crate) fn ssh_parts(&mut self) -> (&std::ffi::CStr, &[u8]) {
        if self.username.last() != Some(&0) {
            self.username.push(0);
        }
        (
            std::ffi::CStr::from_bytes_with_nul(&self.username)
                .expect("parsed username has no controls"),
            &self.password,
        )
    }
    pub(crate) fn header(&self) -> SecretHeader {
        let username = self.username.strip_suffix(&[0]).unwrap_or(&self.username);
        let size = username.len() + self.password.len() + 1;
        let mut credential = SecretBuffer(Vec::with_capacity(size));
        credential.0.extend_from_slice(username);
        credential.0.push(b':');
        credential.0.extend_from_slice(&self.password);
        let encoded_size = base64::encoded_len(size, true).expect("bounded credential");
        let mut header = SecretBuffer(vec![0; 6 + encoded_size]);
        header.0[..6].copy_from_slice(b"Basic ");
        STANDARD
            .encode_slice(&credential.0, &mut header.0[6..])
            .expect("exact encoded size");
        SecretHeader::owned(header)
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(REDACTED)")
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        overwrite(&mut self.username);
        overwrite(&mut self.password);
    }
}

pub(super) fn parse_secret(output: &[u8]) -> Result<Secret, AuthError> {
    if !output.ends_with(b"\n") {
        return Err(AuthError::MissingNewline);
    }
    let text = std::str::from_utf8(output).map_err(|_| AuthError::NotUtf8)?;
    let mut username = None;
    let mut password = None;
    let mut ended = false;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            ended = true;
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some(pair) => pair,
            None if line == "username" || line == "password" => {
                return Err(AuthError::MalformedOutput);
            }
            None => {
                continue;
            }
        };
        if key != "username" && key != "password" {
            continue;
        }
        if ended {
            return Err(AuthError::MalformedOutput);
        }
        if value.chars().any(char::is_control) {
            return Err(AuthError::ControlCharacter);
        }
        let field = if key == "username" {
            &mut username
        } else {
            &mut password
        };
        if field.replace(value.as_bytes()).is_some() {
            return Err(AuthError::MalformedOutput);
        }
    }
    let username = username.ok_or(AuthError::MissingCredential)?;
    let password = password.ok_or(AuthError::MissingCredential)?;
    if username.contains(&b':') {
        return Err(AuthError::UsernameColon);
    }
    // Allocate the SSH terminator slot before copying any credential bytes.
    // ssh_parts never grows/releases a populated allocation.
    let mut owned_username = Vec::with_capacity(username.len() + 1);
    owned_username.extend_from_slice(username);
    Ok(Secret {
        username: owned_username,
        password: password.to_vec(),
    })
}
