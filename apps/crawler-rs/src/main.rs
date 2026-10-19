use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use clap::Parser;
use crawler_rs::runner::{ContinuousConfig, CrawlConfig, run_continuous, run_crawl_with};
use crawler_rs::{SEEDS, database};

#[derive(Debug, Parser)]
#[command(version, about = "Vertex : exploration et indexation du web public")]
struct Args {
    #[arg(default_value_t = 300)]
    duration: u64,
    #[arg(long)]
    continuous: bool,
    /// Process only the existing queue, without discovering more URLs or recrawling old pages.
    #[arg(long)]
    drain: bool,
    #[arg(long)]
    db_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u16).range(1..=64))]
    workers: u16,
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=300))]
    session_secs: u64,
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=3600))]
    idle_secs: u64,
    #[arg(long, default_value_t = 8192, value_parser = clap::value_parser!(u64).range(256..=1_048_576))]
    max_storage_mib: u64,
    #[arg(long, default_value_t = 2048)]
    min_free_mib: u64,
    /// Creating this file requests a graceful stop, without killing the database process.
    #[arg(long)]
    stop_file: Option<PathBuf>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();
    let db_dir = args.db_dir.unwrap_or_else(database::default_db_dir);
    let cfg = CrawlConfig {
        duration_secs: args.duration,
        workers: usize::from(args.workers),
        ..Default::default()
    };
    if args.continuous || args.drain {
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        tokio::spawn(async move {
            #[cfg(unix)]
            {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler");
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
            signal.store(true, Ordering::Release);
        });
        if let Some(path) = args.stop_file {
            if path.exists() {
                println!("Arret demande par {}", path.display());
                return;
            }
            let signal = stop.clone();
            tokio::spawn(async move {
                while !signal.load(Ordering::Acquire) {
                    if path.exists() {
                        signal.store(true, Ordering::Release);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            });
        }
        let Some(min_free_bytes) = args.min_free_mib.checked_mul(1024 * 1024) else {
            eprintln!("Reserve de disque excessive");
            std::process::exit(2);
        };
        let config = ContinuousConfig {
            crawl: CrawlConfig {
                duration_secs: args.session_secs,
                ..cfg
            },
            idle_secs: args.idle_secs,
            max_storage_bytes: args.max_storage_mib * 1024 * 1024,
            min_free_bytes,
            drain_existing: args.drain,
            ..Default::default()
        };
        println!(
            "Vertex {} : {} ; {} workers ; quota {} Mio ; reserve {} Mio",
            env!("CARGO_PKG_VERSION"),
            if args.drain {
                "traitement de la file existante"
            } else {
                "crawl continu"
            },
            args.workers,
            args.max_storage_mib,
            args.min_free_mib
        );
        let worker =
            tokio::task::spawn_blocking(move || run_continuous(&db_dir, SEEDS, config, stop));
        match worker.await {
            Ok(Ok(())) => return,
            result => {
                eprintln!("Arret du crawler : {result:?}");
                std::process::exit(1);
            }
        }
    }
    let summary = tokio::task::spawn_blocking(move || run_crawl_with(&db_dir, SEEDS, cfg))
        .await
        .expect("Crawler thread");
    println!(
        "{} indexees, {} echecs, {} bloquees, {} ignorees, {} en attente ({} s)",
        summary.pages_ok,
        summary.pages_failed,
        summary.pages_blocked,
        summary.pages_skipped,
        summary.pending,
        summary.elapsed_secs
    );
    for error in &summary.errors {
        eprintln!("Erreur du crawl : {error}");
    }
    if !summary.errors.is_empty() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drain_is_explicit_and_limits_are_validated() {
        let args = Args::try_parse_from(["crawler", "--drain", "--workers", "12"]).unwrap();
        assert!(args.drain);
        assert_eq!(args.workers, 12);
        assert!(Args::try_parse_from(["crawler", "--workers", "100"]).is_err());
        assert_eq!(
            Args::try_parse_from(["crawler", "--workers", "64"])
                .unwrap()
                .workers,
            64
        );
        assert!(Args::try_parse_from(["crawler", "--workers", "65"]).is_err());
        assert!(Args::try_parse_from(["crawler", "--unknown"]).is_err());
        assert_eq!(
            Args::try_parse_from(["crawler", "600"]).unwrap().duration,
            600
        );
    }
}
