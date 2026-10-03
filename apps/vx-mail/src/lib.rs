//! vx-mail — messagerie **maison** de Vertex (aucune dépendance).
//!
//! Contient l'encodage MIME nécessaire à un vrai client de courriel :
//! **base64**, **quoted-printable**, analyse des **en-têtes repliés**, séparation
//! **multipart** et composition de messages (RFC 5322). Le transport IMAP/SMTP
//! s'appuiera sur le TLS de `vx-sys`.
//!
//! Conformément à la politique du projet : **aucun journal**, rien n'est
//! transmis ailleurs, les identifiants viennent du coffre-fort (`vx-vault`).

const BASE64_TABLE: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode des octets en base64 (sans retour à la ligne).
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let packed = (b0 << 16) | (b1 << 8) | b2;

        out.push(BASE64_TABLE[((packed >> 18) & 63) as usize] as char);
        out.push(BASE64_TABLE[((packed >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64_TABLE[((packed >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_TABLE[(packed & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Décode du base64 (les espaces et sauts de ligne sont ignorés).
pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;

    for byte in text.bytes() {
        if matches!(byte, b'=' | b'\r' | b'\n' | b' ' | b'\t') {
            continue;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            other => return Err(format!("caractère base64 invalide : {}", other as char)),
        } as u32;

        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
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
