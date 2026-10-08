//! The certificates of the CA file that `GIT_SSL_CAINFO` names, and, where
//! the TLS backend is not OpenSSL, of the one `SSL_CERT_FILE` names (TR2.7).
//! Every `CERTIFICATE` block is a root, which the connector adds beside the
//! platform's built-in roots. The blocks are read here, not by the TLS
//! backend, so that every platform reads a file alike: text outside the
//! blocks is ignored, and a block that holds no certificate, or a file with no
//! block, is refused before any connection opens.
//!
//! Where the backend is OpenSSL, `SSL_CERT_FILE` and `SSL_CERT_DIR` are not CA
//! files of the endpoint but OpenSSL's default verify paths, which are the
//! platform's roots there and are read as OpenSSL reads them, leniently
//! (`super::verify_paths`).
use base64::{Engine as _, engine::general_purpose::STANDARD};

/// Why a CA file is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A certificate block that does not end, whose body is not base64, or
    /// whose content the platform does not take as a certificate.
    Malformed,
    /// No certificate block.
    NoCertificate,
}

/// One block of a PEM file whose label was asked for.
pub(super) enum Block {
    /// The block's body, decoded.
    Der(Vec<u8>),
    /// A block whose body is not base64.
    NotBase64,
}

/// The blocks of a PEM file, in order, and whether the last one never ended.
pub(super) struct Scan {
    pub(super) blocks: Vec<Block>,
    pub(super) unterminated: bool,
}

/// The blocks of `pem` whose label is one of `labels`. Text outside the
/// blocks is ignored, as is a block of any other label.
pub(super) fn scan(pem: &[u8], labels: &[&str]) -> Scan {
    const BEGIN: &[u8] = b"-----BEGIN ";
    const DASHES: &[u8] = b"-----";
    let mut blocks = Vec::new();
    let mut open: Option<(Vec<u8>, Vec<u8>)> = None;
    for line in pem.split(|byte| *byte == b'\n').map(<[u8]>::trim_ascii) {
        match open.as_mut() {
            None => {
                let label = line
                    .strip_prefix(BEGIN)
                    .and_then(|rest| rest.strip_suffix(DASHES));
                if let Some(label) = label
                    && labels.iter().any(|wanted| wanted.as_bytes() == label)
                {
                    let mut end = b"-----END ".to_vec();
                    end.extend_from_slice(label);
                    end.extend_from_slice(DASHES);
                    open = Some((end, Vec::new()));
                }
            }
            Some((end, _)) if line == end.as_slice() => {
                if let Some((_, body)) = open.take() {
                    blocks.push(match STANDARD.decode(body.as_slice()) {
                        Ok(der) => Block::Der(der),
                        Err(_) => Block::NotBase64,
                    });
                }
            }
            Some((_, body)) => {
                body.extend(line.iter().filter(|byte| !byte.is_ascii_whitespace()));
            }
        }
    }
    Scan {
        blocks,
        unterminated: open.is_some(),
    }
}

/// Every certificate in `pem`, the CA file's content, in the file's order.
pub(crate) fn certificates(pem: &[u8]) -> Result<Vec<native_tls::Certificate>, Refusal> {
    let scan = scan(pem, &["CERTIFICATE"]);
    let mut certificates = Vec::new();
    for block in scan.blocks {
        match block {
            Block::Der(der) => certificates
                .push(native_tls::Certificate::from_der(&der).map_err(|_| Refusal::Malformed)?),
            Block::NotBase64 => return Err(Refusal::Malformed),
        }
    }
    if scan.unterminated {
        return Err(Refusal::Malformed);
    }
    if certificates.is_empty() {
        return Err(Refusal::NoCertificate);
    }
    Ok(certificates)
}
