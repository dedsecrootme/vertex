use crawler_rs::runner::{run_crawl_with, CrawlConfig};
use crawler_rs::{database, SEEDS};

/// Durée du crawl lancé en ligne de commande (secondes), surchargeable par
/// l'argument 1 (ex. `cargo run -p crawler-rs -- 600`).
fn main() {
    let duration: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(300);

    println!("============================================");
    println!("Vertex Crawler v{}", env!("CARGO_PKG_VERSION"));
    println!("Duree : {duration}s");
    println!("============================================");
    for seed in SEEDS {
        println!("Seed : {seed}");
    }

    let cfg = CrawlConfig {
        duration_secs: duration,
        ..CrawlConfig::default()
    };

    println!("Demarrage du crawl...");
    let summary = run_crawl_with(&database::default_db_dir(), SEEDS, cfg);

    println!();
    println!("============================================");
    println!("Crawl termine en {}s", summary.elapsed_secs);
    println!("Pages OK      : {}", summary.pages_ok);
    println!("Pages echouees: {}", summary.pages_failed);
    println!("Pages bloquees: {} (malveillantes)", summary.pages_blocked);
    println!("Pages ignorees: {}", summary.pages_skipped);
    println!("Domaines      : {}", summary.sites);
    println!("Encore en file: {}", summary.pending);
    println!("============================================");
}
