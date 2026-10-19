use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use crawler_rs::{database, models::Page, parser};
use sha2::{Digest, Sha256};
use url::Url;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    dir: PathBuf,
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u32).range(1..=10000))]
    pages: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.dir.join("vertex.db").exists() {
        return Err("Benchmark requires a new directory; never use the real index".into());
    }
    let conn = database::init_in(&args.dir)?;
    for id in 0..args.pages {
        database::enqueue(
            &conn,
            &format!("https://benchmark-{}.example/article/{id}", id % 64),
            0,
            0,
        )?;
    }
    let body = "<p>searchablebenchmarkword Rust documentation search indexing public article with a repeatable sample of normal text.</p>".repeat(100);
    let links = (0..12)
        .map(|id| format!("<a href='/guide/{id}'>Guide {id}</a>"))
        .collect::<String>();
    let started = Instant::now();
    for id in 0..args.pages {
        let base = Url::parse(&format!(
            "https://benchmark-{}.example/article/{id}",
            id % 64
        ))?;
        let html = format!(
            "<!doctype html><html lang='en'><head><title>Article {id}</title></head><body>{body}{links}</body></html>"
        );
        let parsed = parser::analyze(&html, &base);
        let page = Page {
            url: base.to_string(),
            domain: base.host_str().unwrap().into(),
            status_code: 200,
            title: parsed.title,
            description: parsed.description,
            content: parsed.text,
            content_html: parsed.sanitized_html,
            links: parsed.links,
            images: parsed.images,
            favicon: parsed.favicon,
            content_hash: format!("{:x}", Sha256::digest(&html)),
            size: html.len(),
            content_type: Some("text/html".into()),
            language: parsed.language,
            depth: 0,
            crawl_time_ms: 0,
        };
        database::save_crawled_page(&conn, &page, 200)?;
    }
    let elapsed = started.elapsed().as_secs_f64();
    let searchable: i64 = conn.query_row(
        "SELECT count(*) FROM pages_fts WHERE pages_fts MATCH 'searchablebenchmarkword'",
        [],
        |row| row.get(0),
    )?;
    if searchable != i64::from(args.pages) {
        return Err("FTS results mismatch".into());
    }
    conn.execute(
        "INSERT INTO pages_fts(pages_fts,rank) VALUES('integrity-check',1)",
        [],
    )?;
    let report = serde_json::json!({
        "kind":"local-html-parse-and-index-no-network", "pages":args.pages,
        "seconds":elapsed, "pages_per_second":f64::from(args.pages)/elapsed,
        "searchable_pages":searchable,"html_bytes_per_page":body.len()+links.len()+100,
        "profile":if cfg!(debug_assertions){"debug"}else{"release"},
        "database":args.dir.join("vertex.db"),
        "note":"Includes HTML parsing, sanitization, page/link storage, FTS5 and completed queue state. Excludes network, DNS, robots, politeness, and queue claiming. Not a live crawl rate."
    });
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(args.dir.join("benchmark.json"), &json)?;
    println!("{json}");
    Ok(())
}
