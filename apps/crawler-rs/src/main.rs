use crawler_rs::{runner::run_crawl, SEEDS};

const CRAWL_SECONDS: u64 = 300;

fn main() {
    println!("============================================");
    println!("🕷️ Vertex Crawler v{}", env!("CARGO_PKG_VERSION"));
    println!("⏱️ Durée : {CRAWL_SECONDS}s");
    println!("============================================");

    for seed in SEEDS {
        println!("➕ Seed : {seed}");
    }

    println!("🏁 Démarrage du crawl…");
    let summary = run_crawl(CRAWL_SECONDS, SEEDS);

    println!();
    println!("============================================");
    println!("✅ Crawl terminé en {}s", summary.elapsed_secs);
    println!("📄 Pages traitées : {} (OK), {} (échec)", summary.pages_ok, summary.pages_failed);
    println!("🌍 Sites (domaines) distincts : {}", summary.sites);
    println!("📥 URLs encore en file : {}", summary.pending);
    println!("============================================");
}