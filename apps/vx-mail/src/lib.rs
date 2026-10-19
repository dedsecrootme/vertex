//! vx-mail — helpers MIME de Vertex.
//!
//! Contains base64 and quoted-printable helpers. Header parsing, multipart,
//! message composition and IMAP/SMTP transport are not implemented yet.
//!
//! Conformément à la politique du projet : **aucun journal**, rien n'est
//! transmis ailleurs, les identifiants viennent du coffre-fort (`vx-vault`).

use base64::Engine;

/// Encode des octets en base64 (sans retour à la ligne).
pub fn base64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Décode du base64 (les espaces et sauts de ligne sont ignorés).
pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let input: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| e.to_string())
}

/// Décode du **quoted-printable** (RFC 2045), y compris les sauts souples.
pub fn quoted_printable_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'=' {
            // Saut de ligne souple : "=" suivi d'un retour.
            if i + 1 < bytes.len() && (bytes[i + 1] == b'\r' || bytes[i + 1] == b'\n') {
                i += if bytes[i + 1] == b'\r' && bytes.get(i + 2) == Some(&b'\n') {
                    3
                } else {
                    2
                };
                continue;
            }
            if i + 2 < bytes.len() {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(decoded) = u8::from_str_radix(hex, 16) {
                    out.push(decoded);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_base64_roundtrips_binary_and_whitespace() {
        let bytes = [0, 255, 128, 42];
        assert_eq!(base64_decode(&base64_encode(&bytes)).unwrap(), bytes);
        assert_eq!(base64_decode("Zm9v\r\nYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn rejects_malformed_base64_instead_of_returning_partial_data() {
        for text in ["A", "AA=A", "Zm9v=garbage", "####", "Zh=="] {
            assert!(base64_decode(text).is_err(), "{text}");
        }
    }

    #[test]
    fn quoted_printable_handles_soft_line_breaks_and_utf8() {
        assert_eq!(
            quoted_printable_decode("caf=C3=A9=\r\n suite"),
            "café suite"
        );
    }
}
