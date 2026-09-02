/// Modèle de données d'une page crawlisée.
///
/// Certains champs (contenu, profondeur, métadonnées, performance) sont
/// prévus pour le futur indexeur et ne sont pas encore exploités.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Page {
    pub url: String,
    pub status_code: u16,
    pub title: String,
    pub description: Option<String>,
    pub content: String,
    pub links: Vec<String>,

    // Métadonnées du crawler
    pub depth: i32,
    pub hash: String,
    pub size: usize,
    pub content_type: Option<String>,
    pub language: Option<String>,

    // Performance
    pub crawl_time_ms: u128,
}