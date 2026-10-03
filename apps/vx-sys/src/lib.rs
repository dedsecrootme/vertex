//! vx-sys — couche « plus bas niveau » de Vertex : **FFI directe** vers les
//! primitives système de Windows (aucune dépendance, aucune crate).
//!
//! - **CNG** (`bcrypt.dll`) : aléa cryptographique, **AES-256-GCM** authentifié.
//! - **SChannel** (`secur32.dll`) : TLS client (voir [`tls`]).
//!
//! La cryptographie n'est jamais réécrite à la main : on pilote les primitives
//! auditées du système, ce qui est le seul choix sûr.

#![allow(non_snake_case)]

use std::ffi::c_void;

pub mod cng;

/// Erreur native (code NTSTATUS en hexadécimal + contexte).
#[derive(Debug, Clone)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn status_error(op: &str, status: i32) -> Error {
    Error(format!("{op} : code 0x{:08X}", status as u32))
}

/// Chaîne large (UTF-16, terminée par NUL) pour les API Windows.
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(alg: *mut c_void, buffer: *mut u8, len: u32, flags: u32) -> i32;
}

/// Octets aléatoires cryptographiques du système (BCryptGenRandom).
/// Panique si le générateur est indisponible — jamais de valeur par défaut.
pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut buffer = vec![0u8; len];
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        panic!("vx-sys : générateur aléatoire système indisponible");
    }
    buffer
}
