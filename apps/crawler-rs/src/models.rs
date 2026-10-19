//! Modèles de données de Vertex (pages, liens complets, images).

/// Lien sortant complet extrait d'une page.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PageLink {
    /// URL absolue complète (jamais tronquée).
    pub url: String,
    /// Texte d'ancre, si présent.
    pub anchor: Option<String>,
    /// Vrai si le lien pointe vers un autre domaine.
    pub is_external: bool,
    /// Verdict de sûreté : `safe` | `suspicious` | `malicious` | `unknown`.
    pub safety: String,
}

/// Image référencée par une page.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PageImage {
    /// URL absolue complète de l'image.
    pub url: String,
    /// Texte alternatif, si présent.
    pub alt: Option<String>,
}

/// Une page crawlisée et indexée.
#[derive(Debug, Clone)]
pub struct Page {
    pub url: String,
    pub domain: String,
    pub status_code: u16,
    pub title: String,
    pub description: Option<String>,
    /// Texte visible nettoyé (indexé par FTS5).
    pub content: String,
    /// HTML assaini pour la visionneuse (sans script, pub, tracker, affiliation).
    pub content_html: String,
    pub links: Vec<PageLink>,
    pub images: Vec<PageImage>,
    pub favicon: Option<String>,
    pub content_hash: String,
    pub size: usize,
    pub content_type: Option<String>,
    pub language: Option<String>,
    pub depth: i32,
    pub crawl_time_ms: u128,
}
