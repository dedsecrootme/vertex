//! Chiffrement authentifié **AES-256-GCM** via CNG (Windows).
//!
//! Pilote `BCryptOpenAlgorithmProvider` / `BCryptSetProperty("ChainingModeGCM")`
//! / `BCryptGenerateSymmetricKey` / `BCryptEncrypt` / `BCryptDecrypt`.
//! Le tag d'authentification (16 octets) protège l'intégrité : toute
//! modification du chiffré, du nonce ou des données associées est détectée.

use std::ffi::c_void;

use crate::{Error, status_error, wide};

/// Taille du nonce recommandée pour GCM (96 bits).
pub const NONCE_LEN: usize = 12;
/// Taille du tag d'authentification (128 bits).
pub const TAG_LEN: usize = 16;
/// Taille de clé (256 bits).
pub const KEY_LEN: usize = 32;

/// `BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO` (structure CNG pour GCM).
#[repr(C)]
struct AuthCipherModeInfo {
    cb_size: u32,
    info_version: u32,
    nonce: *mut u8,
    nonce_len: u32,
    auth_data: *mut u8,
    auth_data_len: u32,
    tag: *mut u8,
    tag_len: u32,
    mac_context: *mut u8,
    mac_context_len: u32,
    aad_len: u32,
    data_len: u64,
    flags: u32,
}

const AUTH_MODE_INFO_VERSION: u32 = 1;

#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptOpenAlgorithmProvider(
        handle: *mut *mut c_void,
        algorithm: *const u16,
        implementation: *const u16,
        flags: u32,
    ) -> i32;

    fn BCryptCloseAlgorithmProvider(handle: *mut c_void, flags: u32) -> i32;

    fn BCryptSetProperty(
        handle: *mut c_void,
        property: *const u16,
        value: *mut u8,
        value_len: u32,
        flags: u32,
    ) -> i32;

    fn BCryptGenerateSymmetricKey(
        algorithm: *mut c_void,
        key: *mut *mut c_void,
        key_object: *mut u8,
        key_object_len: u32,
        secret: *mut u8,
        secret_len: u32,
        flags: u32,
    ) -> i32;

    fn BCryptDestroyKey(key: *mut c_void) -> i32;

    fn BCryptEncrypt(
        key: *mut c_void,
        input: *mut u8,
        input_len: u32,
        info: *mut AuthCipherModeInfo,
        iv: *mut u8,
        iv_len: u32,
        output: *mut u8,
        output_len: u32,
        result: *mut u32,
        flags: u32,
    ) -> i32;

    fn BCryptDecrypt(
        key: *mut c_void,
        input: *mut u8,
        input_len: u32,
        info: *mut AuthCipherModeInfo,
        iv: *mut u8,
        iv_len: u32,
        output: *mut u8,
        output_len: u32,
        result: *mut u32,
        flags: u32,
    ) -> i32;

    fn BCryptDeriveKeyPBKDF2(
        prf: *mut c_void,
        password: *mut u8,
        password_len: u32,
        salt: *mut u8,
        salt_len: u32,
        iterations: u64,
        derived: *mut u8,
        derived_len: u32,
        flags: u32,
    ) -> i32;
}

/// PBKDF2-HMAC-SHA256 dérivé par CNG (`BCryptDeriveKeyPBKDF2`).
/// `iterations` : 600 000 par defaut pour les nouveaux coffres.
pub fn pbkdf2_sha256(
    password: &[u8],
    salt: &[u8],
    iterations: u64,
    out_len: usize,
) -> Result<Vec<u8>, Error> {
    const BCRYPT_ALG_HANDLE_HMAC_FLAG: u32 = 0x0000_0008;
    let alg_name = wide("SHA256");

    let mut prf: *mut c_void = std::ptr::null_mut();
    let status = unsafe {
        BCryptOpenAlgorithmProvider(
            &mut prf,
            alg_name.as_ptr(),
            std::ptr::null(),
            BCRYPT_ALG_HANDLE_HMAC_FLAG,
        )
    };
    if status != 0 {
        return Err(status_error(
            "BCryptOpenAlgorithmProvider(HMAC-SHA256)",
            status,
        ));
    }

    let mut password = zeroize::Zeroizing::new(password.to_vec());
    let mut salt = salt.to_vec();
    let mut derived = vec![0u8; out_len];
    let status = unsafe {
        BCryptDeriveKeyPBKDF2(
            prf,
            password.as_mut_ptr(),
            password.len() as u32,
            salt.as_mut_ptr(),
            salt.len() as u32,
            iterations,
            derived.as_mut_ptr(),
            derived.len() as u32,
            0,
        )
    };
    unsafe { BCryptCloseAlgorithmProvider(prf, 0) };
    if status != 0 {
        return Err(status_error("BCryptDeriveKeyPBKDF2", status));
    }
    Ok(derived)
}

/// Itérations par défaut pour un mot de passe maître.
pub const DEFAULT_PBKDF2_ITERATIONS: u64 = 600_000;

/// Clé symétrique AES-256 en mode GCM.
pub struct AesGcmKey {
    handle: *mut c_void,
}

// Le handle CNG est utilisable depuis n'importe quel thread.
unsafe impl Send for AesGcmKey {}
unsafe impl Sync for AesGcmKey {}

impl Drop for AesGcmKey {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { BCryptDestroyKey(self.handle) };
        }
    }
}

impl AesGcmKey {
    /// Prépare une clé AES-256-GCM à partir de 32 octets de secret.
    pub fn new(key: &[u8; KEY_LEN]) -> Result<Self, Error> {
        let mut algorithm: *mut c_void = std::ptr::null_mut();
        let alg_name = wide("AES");

        let status = unsafe {
            BCryptOpenAlgorithmProvider(&mut algorithm, alg_name.as_ptr(), std::ptr::null(), 0)
        };
        if status != 0 {
            return Err(status_error("BCryptOpenAlgorithmProvider(AES)", status));
        }

        // Mode de chaînage GCM.
        let prop = wide("ChainingMode");
        let mut mode = wide("ChainingModeGCM");
        let mode_bytes = mode.len() * 2;
        let status = unsafe {
            BCryptSetProperty(
                algorithm,
                prop.as_ptr(),
                mode.as_mut_ptr() as *mut u8,
                mode_bytes as u32,
                0,
            )
        };
        if status != 0 {
            unsafe { BCryptCloseAlgorithmProvider(algorithm, 0) };
            return Err(status_error("BCryptSetProperty(GCM)", status));
        }

        let mut handle: *mut c_void = std::ptr::null_mut();
        let mut secret = zeroize::Zeroizing::new(*key);
        let status = unsafe {
            BCryptGenerateSymmetricKey(
                algorithm,
                &mut handle,
                std::ptr::null_mut(),
                0,
                secret.as_mut_ptr(),
                KEY_LEN as u32,
                0,
            )
        };
        unsafe { BCryptCloseAlgorithmProvider(algorithm, 0) };
        if status != 0 {
            return Err(status_error("BCryptGenerateSymmetricKey", status));
        }

        Ok(Self { handle })
    }

    /// Chiffre `plaintext` avec les données associées `aad`.
    /// Renvoie `(chiffré, tag)`.
    pub fn encrypt(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, [u8; TAG_LEN]), Error> {
        let mut tag = [0u8; TAG_LEN];
        let mut aad_buffer = aad.to_vec();
        let mut info = AuthCipherModeInfo {
            cb_size: std::mem::size_of::<AuthCipherModeInfo>() as u32,
            info_version: AUTH_MODE_INFO_VERSION,
            nonce: nonce.as_ptr() as *mut u8,
            nonce_len: NONCE_LEN as u32,
            // Les données associées sont portées par la structure : c'est ce
            // qui les lie au tag d'authentification.
            auth_data: if aad.is_empty() {
                std::ptr::null_mut()
            } else {
                aad_buffer.as_mut_ptr()
            },
            auth_data_len: aad.len() as u32,
            tag: tag.as_mut_ptr(),
            tag_len: TAG_LEN as u32,
            mac_context: std::ptr::null_mut(),
            mac_context_len: 0,
            aad_len: 0,
            data_len: 0,
            flags: 0,
        };

        let mut input = zeroize::Zeroizing::new(plaintext.to_vec());
        let mut output = vec![0u8; plaintext.len()];
        let mut written: u32 = 0;
        let status = unsafe {
            BCryptEncrypt(
                self.handle,
                input.as_mut_ptr(),
                input.len() as u32,
                &mut info,
                std::ptr::null_mut(),
                0,
                output.as_mut_ptr(),
                output.len() as u32,
                &mut written,
                0,
            )
        };
        if status != 0 {
            return Err(status_error("BCryptEncrypt", status));
        }
        output.truncate(written as usize);
        Ok((output, tag))
    }

    /// Déchiffre `ciphertext` et vérifie le tag (16 octets).
    /// Renvoie une erreur si l'authentification échoue.
    pub fn decrypt(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        ciphertext: &[u8],
        tag: &[u8; TAG_LEN],
    ) -> Result<Vec<u8>, Error> {
        let mut tag_buffer = *tag;
        let mut aad_buffer = aad.to_vec();
        let mut info = AuthCipherModeInfo {
            cb_size: std::mem::size_of::<AuthCipherModeInfo>() as u32,
            info_version: AUTH_MODE_INFO_VERSION,
            nonce: nonce.as_ptr() as *mut u8,
            nonce_len: NONCE_LEN as u32,
            auth_data: if aad.is_empty() {
                std::ptr::null_mut()
            } else {
                aad_buffer.as_mut_ptr()
            },
            auth_data_len: aad.len() as u32,
            tag: tag_buffer.as_mut_ptr(),
            tag_len: TAG_LEN as u32,
            mac_context: std::ptr::null_mut(),
            mac_context_len: 0,
            aad_len: 0,
            data_len: 0,
            flags: 0,
        };

        let mut input = ciphertext.to_vec();
        let mut output = vec![0u8; ciphertext.len()];
        let mut written: u32 = 0;
        let status = unsafe {
            BCryptDecrypt(
                self.handle,
                input.as_mut_ptr(),
                input.len() as u32,
                &mut info,
                std::ptr::null_mut(),
                0,
                output.as_mut_ptr(),
                output.len() as u32,
                &mut written,
                0,
            )
        };
        if status != 0 {
            // 0xC000A002 = STATUS_AUTH_TAG_MISMATCH : données ou tag altérés.
            return Err(status_error("BCryptDecrypt (authentification)", status));
        }
        output.truncate(written as usize);
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len() / 2)
            .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Vecteur NIST GCM, cas de test 14 (sans données associées).
    #[test]
    fn nist_gcm_test_case_14() {
        let key = [0u8; 32];
        let cipher = AesGcmKey::new(&key).expect("cle");
        let nonce = [0u8; 12];
        let plaintext = unhex("00000000000000000000000000000000");

        let (ciphertext, tag) = cipher.encrypt(&nonce, b"", &plaintext).expect("encrypt");
        assert_eq!(hex(&ciphertext), "cea7403d4d606b6e074ec5d3baf39d18");
        assert_eq!(hex(&tag), "d0d1c8a799996bf0265b98b5d48ab919");
        assert_eq!(
            cipher
                .decrypt(&nonce, b"", &ciphertext, &tag)
                .expect("decrypt"),
            plaintext
        );
    }

    /// Vecteur NIST GCM, cas de test 15 (avec données associées).
    #[test]
    fn nist_gcm_test_case_15_with_aad() {
        let mut key = [0u8; 32];
        key.copy_from_slice(&unhex(
            "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308",
        ));
        let cipher = AesGcmKey::new(&key).expect("cle");

        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&unhex("cafebabefacedbaddecaf888"));

        let plaintext = unhex(
            "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
        );
        let aad = unhex("feedfacedeadbeeffeedfacedeadbeefabaddad2");

        let (ciphertext, tag) = cipher.encrypt(&nonce, &aad, &plaintext).expect("encrypt");
        assert_eq!(
            hex(&ciphertext),
            "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662"
        );
        assert_eq!(hex(&tag), "76fc6ece0f4e1768cddf8853bb2d551b");
        assert_eq!(
            cipher
                .decrypt(&nonce, &aad, &ciphertext, &tag)
                .expect("decrypt"),
            plaintext
        );
    }

    /// Toute altération est détectée.
    #[test]
    fn tampering_is_rejected() {
        let cipher = AesGcmKey::new(&[7u8; 32]).expect("cle");
        let nonce = [1u8; 12];

        let (mut ciphertext, tag) = cipher
            .encrypt(&nonce, b"entete", b"message secret")
            .expect("encrypt");
        ciphertext[0] ^= 0xFF;
        assert!(
            cipher
                .decrypt(&nonce, b"entete", &ciphertext, &tag)
                .is_err()
        );

        let (ciphertext, tag) = cipher
            .encrypt(&nonce, b"entete", b"message secret")
            .expect("encrypt");
        assert!(
            cipher
                .decrypt(&nonce, b"entete-modifie", &ciphertext, &tag)
                .is_err()
        );
    }
}
