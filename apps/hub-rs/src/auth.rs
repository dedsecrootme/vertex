//! Authentication using maintained password derivation and system randomness.
//!
//! - System randomness through getrandom on every supported platform.
//! - Password derivation: PBKDF2-HMAC-SHA256, 600 000 iterations.
//! - **Jetons de session** : 256 bits d'aléa, encodés en hexadécimal.
//! - Comparaisons **à temps constant** (pas de fuite par timing).

use sha2::Sha256;
use subtle::ConstantTimeEq;

/// Nombre d'itérations PBKDF2.
pub const PBKDF2_ITERATIONS: u32 = 600_000;

/// Octets aléatoires cryptographiques (échec = arrêt immédiat, jamais de zéros).
pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    getrandom::getrandom(&mut buf).expect("Générateur aléatoire du système indisponible");
    buf
}

/// Encode des octets en hexadécimal minuscule.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn hex_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hex = std::str::from_utf8(&bytes[i..i + 2]).unwrap_or("");
        match u8::from_str_radix(hex, 16) {
            Ok(byte) => out.push(byte),
            Err(_) => return Vec::new(),
        }
        i += 2;
    }
    out
}

/// PBKDF2-HMAC-SHA256, longueur dérivée 32 octets (un seul bloc).
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut result = [0; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password, salt, iterations, &mut result);
    result
}

/// Compare deux tranches en temps constant.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}

/// Dérive la représentation stockée d'un mot de passe :
/// `pbkdf2$<itérations>$<sel_hex>$<empreinte_hex>`.
pub fn hash_password(password: &str) -> String {
    let salt = random_bytes(16);
    let derived = pbkdf2_sha256(password.as_bytes(), &salt, PBKDF2_ITERATIONS);
    format!(
        "pbkdf2${}${}${}",
        PBKDF2_ITERATIONS,
        hex(&salt),
        hex(&derived)
    )
}

/// Vérifie un mot de passe contre sa représentation stockée.
pub fn verify_password(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() != 4 || parts[0] != "pbkdf2" {
        return false;
    }
    let Ok(iterations) = parts[1].parse::<u32>() else {
        return false;
    };
    if !(1..=10_000_000).contains(&iterations) {
        return false;
    }
    let salt = hex_decode(parts[2]);
    let expected = hex_decode(parts[3]);
    if salt.is_empty() || expected.len() != 32 {
        return false;
    }
    let derived = pbkdf2_sha256(password.as_bytes(), &salt, iterations);
    constant_time_eq(&derived, &expected)
}

/// Nouveau jeton de session (256 bits).
pub fn new_session_token() -> String {
    hex(&random_bytes(32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_matches_reference_vector() {
        // Vecteur de test RFC 6070 (PBKDF2-HMAC-SHA256, c=1) : P="password", S="salt".
        let derived = pbkdf2_sha256(b"password", b"salt", 1);
        assert_eq!(
            hex(&derived),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
    }

    #[test]
    fn password_roundtrip() {
        let stored = hash_password("s3cret-ultra-long-xyz");
        assert!(verify_password("s3cret-ultra-long-xyz", &stored));
        assert!(!verify_password("mauvais", &stored));
    }

    #[test]
    fn random_tokens_are_unique() {
        assert_ne!(new_session_token(), new_session_token());
    }
}
