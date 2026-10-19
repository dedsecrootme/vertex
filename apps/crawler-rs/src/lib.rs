//! Bibliothèque du crawler Vertex.
//!
//! Modules :
//! - [`crawler`]  : récupération/conversion d'une page ;
//! - [`parser`]   : extraction (titre, liens, images) et assainissement HTML ;
//! - [`safety`]   : vérification anti-malware des liens ;
//! - [`database`] : schéma SQLite, file d'URL, pages, signets, contact ;
//! - [`runner`]   : moteur de crawl complet (robots, sitemaps, politesse) ;
//! - [`models`]   : modèles de données.

pub mod crawler;
pub mod database;
mod frontier;
pub mod models;
pub mod monitor;
pub mod network;
pub mod parser;
pub mod robots;
pub mod runner;
pub mod safety;

/// Graines par défaut : un éventail large de domaines de référence pour amorcer
/// une exploration « partout » (technique, science, actualité, open source…).
pub const SEEDS: &[&str] = &[
    "https://www.rust-lang.org",
    "https://doc.rust-lang.org/book/",
    "https://en.wikipedia.org/wiki/Main_Page",
    "https://fr.wikipedia.org/wiki/Wikip%C3%A9dia:Accueil_principal",
    "https://github.com",
    "https://news.ycombinator.com",
    "https://lobste.rs",
    "https://www.bbc.com",
    "https://www.lemonde.fr",
    "https://www.python.org",
    "https://developer.mozilla.org",
    "https://stackoverflow.com",
    "https://www.gnu.org",
    "https://www.kernel.org",
    "https://lwn.net",
    "https://arxiv.org",
    "https://www.nature.com",
    "https://www.nasa.gov",
    "https://wikimediafoundation.org",
    "https://www.debian.org",
    "https://www.archlinux.org",
    "https://www.freecodecamp.org",
];
