//! Vérification de sûreté des liens (anti-malware / anti-phishing / anti-arnaque).
//!
//! Fonctionne **100 % en local, hors-ligne** :
//! - blocklist embarquée (motifs de domaines connus pour être abusifs) ;
//! - blocklist utilisateur optionnelle (`database/blocklist.txt`, un motif par ligne) ;
//! - heuristiques d'URL (domaine littéral en IP, punycode/homographe, raccourcisseurs,
//!   exécutables téléchargeables, ports inhabituels, identifiants dans l'URL…).
//!
//! Aucune requête réseau n'est effectuée : rien ne fuit vers un tiers.

use std::path::Path;
use std::sync::OnceLock;

use url::Url;

/// Verdict de sûreté d'une URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Safe,
    Suspicious,
    Malicious,
    Unknown,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Safe => "safe",
            Verdict::Suspicious => "suspicious",
            Verdict::Malicious => "malicious",
            Verdict::Unknown => "unknown",
        }
    }
}

/// Résultat détaillé d'une vérification.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SafetyReport {
    pub verdict: Verdict,
    /// Score de risque (0 = anodin). ≥ 30 → suspect, ≥ 70 → malveillant.
    pub score: i32,
    /// Explications lisibles (affichées dans l'interface).
    pub reasons: Vec<String>,
}

impl SafetyReport {
    /// Représentation prête à stocker en base.
    pub fn as_db(&self) -> (String, i64, String) {
        (
            self.verdict.as_str().to_string(),
            self.score as i64,
            self.reasons.join(" ; "),
        )
    }
}

/// Motifs de domaines/marques de fraude fréquemment utilisés.
const BUILTIN_PATTERNS: &[&str] = &[
    "malware",
    "ransomware",
    "keylogger",
    "stealer",
    "botnet",
    "phishing",
    "free-bitcoin",
    "free-crypto",
    "claim-reward",
    "wallet-connect-verify",
];

/// Extensions qui téléchargent du code exécutable (signal fort).
const EXECUTABLE_EXT: &[&str] = &[
    "exe", "scr", "msi", "msp", "bat", "cmd", "com", "pif", "vbs", "vbe", "js", "jse", "wsf",
    "wsh", "hta", "ps1", "psm1", "jar", "dll", "run", "apk", "dmg",
];

/// Archives (contenu inconnu : signal moyen).
const ARCHIVE_EXT: &[&str] = &["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "cab", "iso"];

/// Raccourcisseurs d'URL (destination masquée).
const SHORTENERS: &[&str] = &[
    "bit.ly",
    "tinyurl.com",
    "t.co",
    "goo.gl",
    "is.gd",
    "buff.ly",
    "ow.ly",
    "rebrand.ly",
    "cutt.ly",
    "rb.gy",
    "shorturl.at",
    "t.ly",
    "shorte.st",
    "adf.ly",
];

/// TLD à très forte proportion d'abus (signal faible mais notable).
const ABUSED_TLDS: &[&str] = &["tk", "gq", "ml", "cf"];

static USER_BLOCKLIST: OnceLock<Vec<String>> = OnceLock::new();

/// Blocklist utilisateur déjà chargée (vide tant que `load_user_blocklist` n'a
/// pas été appelée).
pub fn user_blocklist() -> &'static [String] {
    USER_BLOCKLIST.get().map(Vec::as_slice).unwrap_or(&[])
}

/// Charge (une seule fois) la blocklist utilisateur depuis un fichier texte.
/// À appeler au démarrage du crawler/serveur avec `database/blocklist.txt`.
pub fn load_user_blocklist(path: &Path) -> &'static [String] {
    USER_BLOCKLIST
        .get_or_init(|| {
            std::fs::read_to_string(path)
                .map(|content| {
                    content
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty() && !l.starts_with('#'))
                        .map(|l| l.to_ascii_lowercase())
                        .collect()
                })
                .unwrap_or_default()
        })
        .as_slice()
}

fn ext_of(path: &str) -> Option<String> {
    let last = path.rsplit('/').next().unwrap_or(path);
    last.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn registrable_suffix(host: &str) -> String {
    host.rsplit('.').next().unwrap_or(host).to_ascii_lowercase()
}

fn base_domain(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 2 {
        format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        host.to_string()
    }
}

/// Analyse une URL et renvoie un rapport de sûreté (aucune I/O réseau).
pub fn check_url(url: &str) -> SafetyReport {
    let mut score: i32 = 0;
    let mut reasons: Vec<String> = Vec::new();

    let parsed = match Url::parse(url) {
        Ok(u) => u,
        Err(_) => {
            return SafetyReport {
                verdict: Verdict::Malicious,
                score: 100,
                reasons: vec!["URL illisible".to_string()],
            }
        }
    };

    let scheme = parsed.scheme();
    if !matches!(scheme, "http" | "https") {
        score += 100;
        reasons.push(format!("Schéma non web ({scheme})"));
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        score += 40;
        reasons.push("Identifiants insérés dans l'URL (technique d'usurpation)".to_string());
    }

    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let sub = parsed.path().to_ascii_lowercase();

    if host.is_empty() {
        score += 100;
        reasons.push("Hôte absent".to_string());
    }

    if host.parse::<std::net::IpAddr>().is_ok() {
        score += 30;
        reasons.push("Domaine remplacé par une adresse IP brute".to_string());
    }

    if host.starts_with("xn--") || host.contains(".xn--") {
        score += 35;
        reasons.push("Domaine internationalisé (punycode) : risque d'homographe".to_string());
    }

    if host.split('.').count() >= 5 {
        score += 15;
        reasons.push("Nombre de sous-domaines inhabituel".to_string());
    }
    if host.len() > 45 {
        score += 10;
        reasons.push("Domaine anormalement long".to_string());
    }

    if SHORTENERS.contains(&base_domain(&host).as_str()) {
        score += 30;
        reasons.push("Raccourcisseur d'URL : destination masquée".to_string());
    }

    if ABUSED_TLDS.contains(&registrable_suffix(&host).as_str()) {
        score += 15;
        reasons.push("Extension de domaine fréquemment détournée".to_string());
    }

    let mut blocked = false;
    for pattern in BUILTIN_PATTERNS {
        if host.contains(pattern) || sub.contains(pattern) {
            // Un motif frauduleux connu suffit à classer l'URL comme malveillante.
            score += 70;
            reasons.push(format!("Motif frauduleux connu : « {pattern} »"));
            blocked = true;
            break;
        }
    }
    if !blocked {
        let lowered = url.to_ascii_lowercase();
        for pattern in user_blocklist() {
            if host.contains(pattern.as_str()) || lowered.contains(pattern.as_str()) {
                score += 70;
                reasons.push(format!("Présent dans la blocklist utilisateur : « {pattern} »"));
                break;
            }
        }
    }

    if let Some(ext) = ext_of(parsed.path()) {
        if EXECUTABLE_EXT.contains(&ext.as_str()) {
            score += 45;
            reasons.push(format!("Téléchargement de code exécutable (.{ext})"));
        } else if ARCHIVE_EXT.contains(&ext.as_str()) {
            score += 20;
            reasons.push(format!("Archive téléchargeable (.{ext})"));
        }
    }

    if let Some(port) = parsed.port() {
        if port != 80 && port != 443 {
            score += 10;
            reasons.push(format!("Port réseau inhabituel ({port})"));
        }
    }

    let verdict = if score >= 70 {
        Verdict::Malicious
    } else if score >= 30 {
        Verdict::Suspicious
    } else {
        Verdict::Safe
    };

    SafetyReport {
        verdict,
        score,
        reasons,
    }
}

/// Vrai si le verdict est « malveillant » (à ne pas suivre du tout).
pub fn is_malicious(report: &SafetyReport) -> bool {
    report.verdict == Verdict::Malicious
}

/// Ensemble des verdicts considérés comme affichables sans avertissement bloquant.
pub fn is_trustworthy(verdict: &str) -> bool {
    matches!(verdict, "safe" | "unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_executable_download() {
        assert_ne!(check_url("http://example.com/setup.exe").verdict, Verdict::Safe);
    }

    #[test]
    fn flags_ip_host_and_shortener() {
        assert_ne!(check_url("http://192.168.1.10/login").verdict, Verdict::Safe);
        assert_eq!(check_url("https://bit.ly/abc").verdict, Verdict::Suspicious);
    }

    #[test]
    fn accepts_normal_url() {
        let r = check_url("https://www.rust-lang.org/learn");
        assert_eq!(r.verdict, Verdict::Safe);
        assert!(r.reasons.is_empty());
    }

    #[test]
    fn flags_phishing_keyword() {
        assert_eq!(
            check_url("https://secure-login-phishing.example/a").verdict,
            Verdict::Malicious
        );
    }
}
