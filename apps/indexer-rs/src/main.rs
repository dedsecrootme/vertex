use rusqlite::Connection;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(manifest_dir)
        .to_path_buf()
}

fn database_path() -> PathBuf {
    workspace_root()
        .join("database")
        .join("vertex.db")
}

fn main() {
    println!("============================================");
    println!("Vertex Indexer v{}", env!("CARGO_PKG_VERSION"));
    println!("============================================");

    let db_path = database_path();

    println!("Base de donnees : {}", db_path.display());

    if !db_path.exists() {
        eprintln!("ERREUR : la base Vertex n'existe pas.");
        eprintln!("Lance d'abord le crawler pour initialiser la base.");
        std::process::exit(1);
    }

    let conn = match Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("ERREUR : impossible d'ouvrir SQLite : {error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = conn.execute_batch("PRAGMA journal_mode = WAL;") {
        eprintln!("ERREUR SQLite : {error}");
        std::process::exit(1);
    }

    println!("Connexion SQLite OK.");

    let pages_count: i64 = match conn.query_row(
        "SELECT COUNT(*) FROM pages",
        [],
        |row| row.get(0),
    ) {
        Ok(count) => count,
        Err(error) => {
            eprintln!("ERREUR : impossible de lire la table pages : {error}");
            std::process::exit(1);
        }
    };

    println!("Pages presentes : {pages_count}");

    if let Err(error) = ensure_fts(&conn) {
        eprintln!("ERREUR : impossible de preparer FTS5 : {error}");
        std::process::exit(1);
    }

    let indexed_count: i64 = match conn.query_row(
        "SELECT COUNT(*) FROM pages_fts",
        [],
        |row| row.get(0),
    ) {
        Ok(count) => count,
        Err(error) => {
            eprintln!("ERREUR : impossible de verifier pages_fts : {error}");
            std::process::exit(1);
        }
    };

    println!("Documents indexes : {indexed_count}");

    println!();
    println!("Indexation terminee.");
}

fn ensure_fts(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
            url UNINDEXED,
            title,
            content,
            content='pages',
            content_rowid='id'
        );

        CREATE TRIGGER IF NOT EXISTS pages_fts_ai
        AFTER INSERT ON pages BEGIN
            INSERT INTO pages_fts(rowid, url, title, content)
            VALUES (new.id, new.url, new.title, new.content);
        END;

        CREATE TRIGGER IF NOT EXISTS pages_fts_ad
        AFTER DELETE ON pages BEGIN
            INSERT INTO pages_fts(pages_fts, rowid, url, title, content)
            VALUES ('delete', old.id, old.url, old.title, old.content);
        END;

        CREATE TRIGGER IF NOT EXISTS pages_fts_au
        AFTER UPDATE ON pages BEGIN
            INSERT INTO pages_fts(pages_fts, rowid, url, title, content)
            VALUES ('delete', old.id, old.url, old.title, old.content);

            INSERT INTO pages_fts(rowid, url, title, content)
            VALUES (new.id, new.url, new.title, new.content);
        END;
        ",
    )?;

    let indexed_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM pages_fts", [], |row| row.get(0))?;

    let pages_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))?;

    if indexed_count != pages_count {
        println!("Reconstruction de l'index FTS5...");

        conn.execute(
            "INSERT INTO pages_fts(pages_fts) VALUES('rebuild')",
            [],
        )?;

        println!("Index FTS5 reconstruit.");
    }

    Ok(())
}