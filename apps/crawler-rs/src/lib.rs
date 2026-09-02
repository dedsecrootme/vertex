//! Bibliothèque du crawler Vertex : extraction, base SQLite, modèle et parseur.
pub mod crawler;
pub mod database;
pub mod models;
pub mod parser;
pub mod runner;

/// URLs de départ utilisées par défaut pour les crawls.
pub const SEEDS: &[&str] = &[
    "https://www.rust-lang.org",
    "https://en.wikipedia.org/wiki/Main_Page",
    "https://www.wikipedia.org",
    "https://github.com",
    "https://news.ycombinator.com",
    "https://www.bbc.com",
    "https://www.python.org",
    "https://developer.mozilla.org",
    "https://stackoverflow.com",
    "https://www.gnu.org",
    "https://www.kernel.org",
    "https://lwn.net",
];