//! Bounded encoded HTTPS selector validation, without allocating decoded copies.
pub(super) fn has_decoded_control(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut cursor = 0;
    let mut utf8 = [0; 4];
    let mut used = 0;
    let mut invalid = false;
    let mut unicode_control = false;
    while cursor < bytes.len() {
        let byte = if bytes[cursor] == b'%' && cursor + 2 < bytes.len() {
            match (hex(bytes[cursor + 1]), hex(bytes[cursor + 2])) {
                (Some(a), Some(b)) => {
                    cursor += 3;
                    a * 16 + b
                }
                _ => {
                    cursor += 1;
                    b'%'
                }
            }
        } else {
            let byte = bytes[cursor];
            cursor += 1;
            byte
        };
        if byte.is_ascii_control() {
            return true;
        }
        if invalid {
            continue;
        }
        utf8[used] = byte;
        used += 1;
        match std::str::from_utf8(&utf8[..used]) {
            Ok(text) => {
                unicode_control |= text.chars().any(char::is_control);
                used = 0;
            }
            Err(error) if error.error_len().is_some() || used == 4 => {
                invalid = true;
            }
            Err(_) => {}
        }
    }
    unicode_control && !invalid && used == 0
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
