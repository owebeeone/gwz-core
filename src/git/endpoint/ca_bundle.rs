//! The certificates of the CA file that `GIT_SSL_CAINFO` or `SSL_CERT_FILE`
//! names (TR2.7). Every `CERTIFICATE` block is a root, which the connector
//! adds beside the platform's built-in roots. The blocks are read here, not by
//! the TLS backend, so that every platform reads a file alike: text outside
//! the blocks is ignored, and a block that holds no certificate, or a file
//! with no block, is refused before any connection opens.
use base64::{Engine as _, engine::general_purpose::STANDARD};

const BEGIN: &[u8] = b"-----BEGIN CERTIFICATE-----";
const END: &[u8] = b"-----END CERTIFICATE-----";

/// Why a CA file is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A certificate block that does not end, whose body is not base64, or
    /// whose content the platform does not take as a certificate.
    Malformed,
    /// No certificate block.
    NoCertificate,
}

/// Every certificate in `pem`, the CA file's content, in the file's order.
pub(crate) fn certificates(pem: &[u8]) -> Result<Vec<native_tls::Certificate>, Refusal> {
    let mut certificates = Vec::new();
    let mut block: Option<Vec<u8>> = None;
    for line in pem.split(|byte| *byte == b'\n').map(<[u8]>::trim_ascii) {
        match block.as_mut() {
            None => {
                if line == BEGIN {
                    block = Some(Vec::new());
                }
            }
            Some(body) if line == END => {
                let der = STANDARD
                    .decode(body.as_slice())
                    .map_err(|_| Refusal::Malformed)?;
                certificates
                    .push(native_tls::Certificate::from_der(&der).map_err(|_| Refusal::Malformed)?);
                block = None;
            }
            Some(body) => {
                body.extend(line.iter().filter(|byte| !byte.is_ascii_whitespace()));
            }
        }
    }
    if block.is_some() {
        return Err(Refusal::Malformed);
    }
    if certificates.is_empty() {
        return Err(Refusal::NoCertificate);
    }
    Ok(certificates)
}
