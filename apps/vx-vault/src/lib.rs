//! vx-vault — le **coffre-fort** de Vertex.
//!
//! - Mot de passe maître → clé via **PBKDF2-HMAC-SHA256** (CNG, 200 000 itérations) ;
//! - Secrets chiffrés en **AES-256-GCM** avec **en-tête authentifié** (toute
//!   altération du fichier — sel, itérations, chiffré — est détectée) ;
//! - **Génération de mots de passe ultra-longs** (256 bits) ;
//! - Secrets gardés **en mémoire uniquement**, verrouillage automatique.
//!
//! Aucune dépendance : toute la cryptographie vient de `vx-sys` (CNG/Windows).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vx_sys::cng::{self, AesGcmKey, KEY_LEN, NONCE_LEN, TAG_LEN};

/// Signature du fichier de coffre.
const MAGIC: &[u8; 8] = b"VXVLT001";
const VERSION: u8 = 1;
const SALT_LEN: usize = 16;
/// Taille de l'en-tête authentifié : magic + version + itérations + sel.
const HEADER_LEN: usize = 8 + 1 + 8 + SALT_LEN;

/// Délai d'inactivité par défaut avant verrouillage automatique.
pub const DEFAULT_AUTO_LOCK: Duration = Duration::from_secs(300);

/// Alphabet sans caractères ambigus, pour les mots de passe générés.
const PASSWORD_ALPHABET: &[u8] =
    b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%^&*-_=+?";

/// Coffre-fort déverrouillé.
pub struct Vault {
    path: PathBuf,
    key: AesGcmKey,
    salt: [u8; SALT_LEN],
    iterations: u64,
    entries: BTreeMap<String, String>,
    last_use: Instant,
    auto_lock: Duration,
}

impl Vault {
    /// Crée un nouveau coffre protégé par `passphrase`.
    pub fn create(path: &Path, passphrase: &str) -> Result<Self, String> {
        if passphrase.chars().count() < 8 {
            return Err("mot de passe maître trop court (8 caractères minimum)".into());
        }
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&vx_sys::random_bytes(SALT_LEN));

        let key = derive_key(passphrase, &salt, cng::DEFAULT_PBKDF2_ITERATIONS)?;
        let vault = Self {
            path: path.to_path_buf(),
            key,
            salt,
            iterations: cng::DEFAULT_PBKDF2_ITERATIONS,
            entries: BTreeMap::new(),
            last_use: Instant::now(),
            auto_lock: DEFAULT_AUTO_LOCK,
        };
        vault.save()?;
        Ok(vault)
    }

    /// Ouvre un coffre existant (échoue si le mot de passe est faux).
    pub fn open(path: &Path, passphrase: &str) -> Result<Self, String> {
        let raw = std::fs::read(path).map_err(|e| format!("lecture du coffre : {e}"))?;
        if raw.len() < HEADER_LEN + NONCE_LEN + TAG_LEN {
            return Err("fichier de coffre tronqué".into());
        }
        if &raw[..8] != MAGIC {
            return Err("ce fichier n'est pas un coffre Vertex".into());
        }
        if raw[8] != VERSION {
            return Err(format!("version de coffre non supportée ({})", raw[8]));
        }

        let mut iterations_bytes = [0u8; 8];
        iterations_bytes.copy_from_slice(&raw[9..17]);
        let iterations = u64::from_le_bytes(iterations_bytes);

        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&raw[17..17 + SALT_LEN]);

        let key = derive_key(passphrase, &salt, iterations)?;

        // L'en-tête entier est authentifié : le modifier invalide la clé.
        let header = &raw[..HEADER_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&raw[HEADER_LEN..HEADER_LEN + NONCE_LEN]);

        let tag_start = raw.len() - TAG_LEN;
        let ciphertext = &raw[HEADER_LEN + NONCE_LEN..tag_start];
        let mut tag = [0u8; TAG_LEN];
        tag.copy_from_slice(&raw[tag_start..]);

        let plaintext = key
            .decrypt(&nonce, header, ciphertext, &tag)
            .map_err(|_| "mot de passe maître incorrect ou coffre altéré".to_string())?;

        Ok(Self {
            path: path.to_path_buf(),
            key,
            salt,
            entries: decode_entries(&plaintext)?,
            iterations,
            last_use: Instant::now(),
            auto_lock: DEFAULT_AUTO_LOCK,
        })
    }

    /// Chemin du fichier de coffre.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Chiffre et écrit le coffre sur disque (écriture atomique via `.tmp`).
    pub fn save(&self) -> Result<(), String> {
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&vx_sys::random_bytes(NONCE_LEN));

        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(MAGIC);
        header.push(VERSION);
        header.extend_from_slice(&self.iterations.to_le_bytes());
        header.extend_from_slice(&self.salt);

        let plaintext = encode_entries(&self.entries);
        let (ciphertext, tag) = self
            .key
            .encrypt(&nonce, &header, &plaintext)
            .map_err(|e| format!("chiffrement : {e}"))?;

        let mut out = Vec::with_capacity(HEADER_LEN + NONCE_LEN + ciphertext.len() + TAG_LEN);
        out.extend_from_slice(&header);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        out.extend_from_slice(&tag);

        let temporary = self.path.with_extension("tmp");
        {
            let mut file =
                std::fs::File::create(&temporary).map_err(|e| format!("création : {e}"))?;
            file.write_all(&out).map_err(|e| format!("écriture : {e}"))?;
            file.sync_all().ok();
        }
        std::fs::rename(&temporary, &self.path).map_err(|e| format!("remplacement : {e}"))?;
        Ok(())
    }

    /// Variante de [`Vault::create`] avec un nombre d'itérations explicite
    /// (réservé aux tests ; garder la valeur par défaut en production).
    pub fn create_with_iterations(
        path: &Path,
        passphrase: &str,
        iterations: u64,
    ) -> Result<Self, String> {
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&vx_sys::random_bytes(SALT_LEN));
        let key = derive_key(passphrase, &salt, iterations)?;
        let vault = Self {
            path: path.to_path_buf(),
            key,
            salt,
            iterations,
            entries: BTreeMap::new(),
            last_use: Instant::now(),
            auto_lock: DEFAULT_AUTO_LOCK,
        };
        vault.save()?;
        Ok(vault)
    }

    // --- Secrets ---------------------------------------------------------

    /// Enregistre (ou remplace) un secret et enregistre le coffre.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.touch();
        self.entries.insert(key.to_string(), value.to_string());
        self.save()
    }

    /// Lit un secret (met à jour l'horodatage d'activité).
    pub fn get(&mut self, key: &str) -> Option<&str> {
        self.touch();
        self.entries.get(key).map(String::as_str)
    }

    /// Supprime un secret.
    pub fn remove(&mut self, key: &str) -> Result<(), String> {
        self.touch();
        self.entries.remove(key);
        self.save()
    }

    /// Noms des secrets stockés (jamais les valeurs).
    pub fn keys(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }

    /// Nombre de secrets.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // --- Mots de passe générés -------------------------------------------

    /// Génère un mot de passe **ultra-long** de `length` caractères
    /// (aléa cryptographique système, alphabet sans caractères ambigus).
    pub fn generate_password(&self, length: usize) -> String {
        let length = length.max(16);
        let mut out = String::with_capacity(length);
        // Rejet pour éviter tout biais modulo.
        let limit = (256 / PASSWORD_ALPHABET.len()) * PASSWORD_ALPHABET.len();
        while out.len() < length {
            for byte in vx_sys::random_bytes(length) {
                if (byte as usize) < limit {
                    out.push(PASSWORD_ALPHABET[byte as usize % PASSWORD_ALPHABET.len()] as char);
                    if out.len() == length {
                        break;
                    }
                }
            }
        }
        out
    }

    // --- Verrouillage ------------------------------------------------------

    /// Réglage du délai de verrouillage automatique.
    pub fn set_auto_lock(&mut self, delay: Duration) {
        self.auto_lock = delay;
    }

    /// Marque une activité (repousse le verrouillage automatique).
    pub fn touch(&mut self) {
        self.last_use = Instant::now();
    }

    /// Vrai si le délai d'inactivité est dépassé.
    pub fn is_expired(&self) -> bool {
        self.last_use.elapsed() >= self.auto_lock
    }

    /// Verrouille : efface les secrets de la mémoire et détruit la clé.
    pub fn lock(mut self) {
        for value in self.entries.values_mut() {
            // Écrasement mémoire best-effort avant libération.
            unsafe { value.as_mut_vec().fill(0) };
        }
        self.entries.clear();
    }
}

/// Encodage des entrées : `[u32 longueur][octets]` pour la clé puis la valeur.
fn encode_entries(entries: &BTreeMap<String, String>) -> Vec<u8> {
    let mut out = Vec::new();
    for (key, value) in entries {
        push_len_prefixed(&mut out, key.as_bytes());
        push_len_prefixed(&mut out, value.as_bytes());
    }
    out
}

fn push_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn take_len_prefixed<'a>(data: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], String> {
    if *cursor + 4 > data.len() {
        return Err("coffre corrompu (longueur)".into());
    }
    let mut length_bytes = [0u8; 4];
    length_bytes.copy_from_slice(&data[*cursor..*cursor + 4]);
    *cursor += 4;
    let length = u32::from_le_bytes(length_bytes) as usize;
    if *cursor + length > data.len() {
        return Err("coffre corrompu (contenu)".into());
    }
    let slice = &data[*cursor..*cursor + length];
    *cursor += length;
    Ok(slice)
}

fn decode_entries(data: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let mut entries = BTreeMap::new();
    let mut cursor = 0usize;
    while cursor < data.len() {
        let key = take_len_prefixed(data, &mut cursor)?;
        let value = take_len_prefixed(data, &mut cursor)?;
        let key = String::from_utf8(key.to_vec()).map_err(|_| "clé non UTF-8".to_string())?;
        let value = String::from_utf8(value.to_vec()).map_err(|_| "valeur non UTF-8".to_string())?;
        entries.insert(key, value);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Itérations réduites : les tests doivent rester rapides.
    const TEST_ITERATIONS: u64 = 1_000;

    fn temp_path() -> PathBuf {
        let mut name = String::from("vx-vault-test-");
        for byte in vx_sys::random_bytes(8) {
            name.push_str(&format!("{byte:02x}"));
        }
        std::env::temp_dir().join(name + ".vxv")
    }

    #[test]
    fn secrets_roundtrip_through_disk() {
        let path = temp_path();
        let mut vault =
            Vault::create_with_iterations(&path, "phrase-maitre-de-test", TEST_ITERATIONS).unwrap();
        vault.set("imap", "mot-de-passe-ultra-long-256bits").unwrap();
        vault.set("smtp", "un-autre-secret").unwrap();
        assert_eq!(vault.len(), 2);
        drop(vault);

        let mut reopened = Vault::open(&path, "phrase-maitre-de-test").unwrap();
        assert_eq!(reopened.get("imap"), Some("mot-de-passe-ultra-long-256bits"));
        assert_eq!(reopened.keys().len(), 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn wrong_master_passphrase_is_rejected() {
        let path = temp_path();
        let mut vault = Vault::create_with_iterations(&path, "bonne-phrase", TEST_ITERATIONS).unwrap();
        vault.set("cle", "valeur").unwrap();
        drop(vault);
        assert!(Vault::open(&path, "mauvaise-phrase").is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn tampered_vault_is_detected() {
        let path = temp_path();
        let mut vault = Vault::create_with_iterations(&path, "phrase", TEST_ITERATIONS).unwrap();
        vault.set("cle", "valeur-secrete").unwrap();
        drop(vault);

        let mut raw = std::fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0x01; // altération du tag d'authentification
        std::fs::write(&path, &raw).unwrap();
        assert!(Vault::open(&path, "phrase").is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn generated_passwords_are_long_unique_and_in_alphabet() {
        let path = temp_path();
        let vault = Vault::create_with_iterations(&path, "phrase", TEST_ITERATIONS).unwrap();
        let first = vault.generate_password(64);
        let second = vault.generate_password(64);
        assert_eq!(first.chars().count(), 64);
        assert_ne!(first, second);
        assert!(first.bytes().all(|b| PASSWORD_ALPHABET.contains(&b)));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn auto_lock_expires_after_inactivity() {
        let path = temp_path();
        let mut vault = Vault::create_with_iterations(&path, "phrase", TEST_ITERATIONS).unwrap();
        vault.set_auto_lock(Duration::from_millis(10));
        vault.touch(); // la dérivation de clé a déjà consommé du temps
        assert!(!vault.is_expired());
        std::thread::sleep(Duration::from_millis(25));
        assert!(vault.is_expired());
        std::fs::remove_file(&path).ok();
    }
}

/// Dérive une clé AES-256 depuis la phrase maître et le sel.
fn derive_key(passphrase: &str, salt: &[u8], iterations: u64) -> Result<AesGcmKey, String> {
    let derived = cng::pbkdf2_sha256(passphrase.as_bytes(), salt, iterations, KEY_LEN)
        .map_err(|e| format!("dérivation de clé : {e}"))?;
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&derived);
    AesGcmKey::new(&key).map_err(|e| format!("clé AES : {e}"))
}
