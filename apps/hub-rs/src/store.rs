//! Stockage du service communautaire (`hub.db`, séparé de l'index local).
//!
//! Contient : comptes, sessions, salons de chat + messages, forums
//! (catégories → sujets → réponses), avis sur le logiciel, et partenaires
//! (publicité de serveur Discord, payante et modérée).

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Result};
use serde::Serialize;

/// Version du schéma du hub.
pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'user',
    created_at    DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sessions (
    token      TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);

CREATE TABLE IF NOT EXISTS rooms (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    slug       TEXT UNIQUE NOT NULL,
    name       TEXT NOT NULL,
    topic      TEXT,
    created_by INTEGER REFERENCES users(id),
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS messages (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    room_id    INTEGER NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id),
    body       TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_messages_room ON messages(room_id, id);

CREATE TABLE IF NOT EXISTS categories (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    slug        TEXT UNIQUE NOT NULL,
    name        TEXT NOT NULL,
    description TEXT
);

CREATE TABLE IF NOT EXISTS topics (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    category_id  INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
    user_id      INTEGER NOT NULL REFERENCES users(id),
    title        TEXT NOT NULL,
    body         TEXT NOT NULL,
    created_at   DATETIME DEFAULT CURRENT_TIMESTAMP,
    last_post_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_topics_cat ON topics(category_id, last_post_at DESC);

CREATE TABLE IF NOT EXISTS posts (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    topic_id   INTEGER NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id),
    body       TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_posts_topic ON posts(topic_id, id);

CREATE TABLE IF NOT EXISTS reviews (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id    INTEGER UNIQUE NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rating     INTEGER NOT NULL CHECK(rating BETWEEN 1 AND 5),
    comment    TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS ads (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_id    INTEGER NOT NULL REFERENCES users(id),
    title       TEXT NOT NULL,
    invite_url  TEXT NOT NULL,
    description TEXT,
    status      TEXT NOT NULL DEFAULT 'pending',
    paid        INTEGER NOT NULL DEFAULT 0,
    starts_at   DATETIME,
    ends_at     DATETIME,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_ads_status ON ads(status);
"#;

/// Racine du workspace Vertex.
fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(manifest_dir)
        .to_path_buf()
}

/// Chemin par défaut de la base du hub.
pub fn default_path() -> PathBuf {
    workspace_root().join("database").join("hub.db")
}

/// Ouvre (et initialise) la base du hub.
pub fn init(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(15))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;",
    )?;
    conn.execute_batch(SCHEMA)?;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [SCHEMA_VERSION.to_string()],
    )?;
    seed_defaults(&conn)?;
    Ok(conn)
}

/// Crée les salons et catégories de départ (idempotent).
fn seed_defaults(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO rooms(slug, name, topic) VALUES ('general', 'Général', 'Discussion libre')",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO rooms(slug, name, topic) VALUES ('aide', 'Aide', 'Entraide et questions')",
        [],
    )?;
    for (slug, name, description) in [
        ("annonces", "Annonces", "Nouveautés de Vertex"),
        ("idees", "Idées & suggestions", "Proposez des fonctionnalités"),
        ("bugs", "Bugs", "Signalez un problème"),
        ("discussion", "Discussion", "Tout et rien"),
    ] {
        conn.execute(
            "INSERT OR IGNORE INTO categories(slug, name, description) VALUES (?1, ?2, ?3)",
            rusqlite::params![slug, name, description],
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Comptes et sessions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub created_at: Option<String>,
}

/// Crée un compte. Renvoie une erreur si le pseudo est déjà pris.
pub fn create_user(conn: &Connection, username: &str, password_hash: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO users(username, password_hash) VALUES (?1, ?2)",
        rusqlite::params![username, password_hash],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Cherche un compte par pseudo (avec son empreinte de mot de passe).
pub fn find_user(conn: &Connection, username: &str) -> Result<Option<(User, String)>> {
    conn.query_row(
        "SELECT id, username, role, created_at, password_hash FROM users WHERE username = ?1",
        [username],
        |row| {
            Ok((
                User {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    role: row.get(2)?,
                    created_at: row.get(3)?,
                },
                row.get::<_, String>(4)?,
            ))
        },
    )
    .optional()
}

/// Compte par identifiant.
pub fn user_by_id(conn: &Connection, id: i64) -> Result<Option<User>> {
    conn.query_row(
        "SELECT id, username, role, created_at FROM users WHERE id = ?1",
        [id],
        |row| {
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                role: row.get(2)?,
                created_at: row.get(3)?,
            })
        },
    )
    .optional()
}

/// Ouvre une session valable `days` jours et renvoie son jeton.
pub fn create_session(conn: &Connection, user_id: i64, token: &str, days: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO sessions(token, user_id, expires_at)
         VALUES (?1, ?2, datetime('now', ?3))",
        rusqlite::params![token, user_id, format!("+{days} days")],
    )?;
    Ok(())
}

/// Renvoie l'utilisateur associé à un jeton de session valide.
pub fn user_for_token(conn: &Connection, token: &str) -> Result<Option<User>> {
    conn.query_row(
        "SELECT u.id, u.username, u.role, u.created_at
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token = ?1 AND s.expires_at > datetime('now')",
        [token],
        |row| {
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                role: row.get(2)?,
                created_at: row.get(3)?,
            })
        },
    )
    .optional()
}

/// Ferme une session.
pub fn delete_session(conn: &Connection, token: &str) -> Result<()> {
    conn.execute("DELETE FROM sessions WHERE token = ?1", [token])?;
    Ok(())
}

/// Supprime les sessions expirées.
pub fn purge_sessions(conn: &Connection) -> Result<usize> {
    Ok(conn.execute("DELETE FROM sessions WHERE expires_at <= datetime('now')", [])?)
}

/// Nombre de comptes (sert de seuil pour l'ouverture des avis).
pub fn count_users(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
}

// ---------------------------------------------------------------------------
// Chat (salons + messages)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Room {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub topic: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: i64,
    pub username: String,
    pub body: String,
    pub created_at: Option<String>,
}

/// Tous les salons.
pub fn list_rooms(conn: &Connection) -> Result<Vec<Room>> {
    let mut stmt = conn.prepare("SELECT id, slug, name, topic FROM rooms ORDER BY id")?;
    let rows = stmt.query_map([], |row| {
        Ok(Room {
            id: row.get(0)?,
            slug: row.get(1)?,
            name: row.get(2)?,
            topic: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Salon par slug.
pub fn room_by_slug(conn: &Connection, slug: &str) -> Result<Option<Room>> {
    conn.query_row(
        "SELECT id, slug, name, topic FROM rooms WHERE slug = ?1",
        [slug],
        |row| {
            Ok(Room {
                id: row.get(0)?,
                slug: row.get(1)?,
                name: row.get(2)?,
                topic: row.get(3)?,
            })
        },
    )
    .optional()
}

/// Crée un salon.
pub fn create_room(
    conn: &Connection,
    slug: &str,
    name: &str,
    topic: &str,
    user_id: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO rooms(slug, name, topic, created_by) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![slug, name, topic, user_id],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Messages d'un salon (croissant, `since_id` pour le rafraîchissement).
pub fn list_messages(
    conn: &Connection,
    room_id: i64,
    since_id: i64,
    limit: i64,
) -> Result<Vec<Message>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, u.username, m.body, m.created_at
         FROM messages m JOIN users u ON u.id = m.user_id
         WHERE m.room_id = ?1 AND m.id > ?2
         ORDER BY m.id ASC LIMIT ?3",
    )?;
    let rows = stmt.query_map(rusqlite::params![room_id, since_id, limit], |row| {
        Ok(Message {
            id: row.get(0)?,
            username: row.get(1)?,
            body: row.get(2)?,
            created_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Publie un message.
pub fn post_message(conn: &Connection, room_id: i64, user_id: i64, body: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO messages(room_id, user_id, body) VALUES (?1, ?2, ?3)",
        rusqlite::params![room_id, user_id, body],
    )?;
    Ok(conn.last_insert_rowid())
}

// ---------------------------------------------------------------------------
// Forums (catégories → sujets → réponses)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Category {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub topics: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Topic {
    pub id: i64,
    pub title: String,
    pub username: String,
    pub posts: i64,
    pub last_post_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Post {
    pub id: i64,
    pub username: String,
    pub body: String,
    pub created_at: Option<String>,
}

/// Catégories avec le nombre de sujets.
pub fn list_categories(conn: &Connection) -> Result<Vec<Category>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.slug, c.name, c.description,
                (SELECT COUNT(*) FROM topics t WHERE t.category_id = c.id)
         FROM categories c ORDER BY c.id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Category {
            id: row.get(0)?,
            slug: row.get(1)?,
            name: row.get(2)?,
            description: row.get(3)?,
            topics: row.get(4)?,
        })
    })?;
    rows.collect()
}

/// Identifiant d'une catégorie à partir de son slug.
pub fn category_id(conn: &Connection, slug: &str) -> Result<Option<i64>> {
    conn.query_row("SELECT id FROM categories WHERE slug = ?1", [slug], |r| r.get(0))
        .optional()
}

/// Sujets d'une catégorie (les plus récents d'abord).
pub fn list_topics(conn: &Connection, category_id: i64) -> Result<Vec<Topic>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.title, u.username,
                (SELECT COUNT(*) FROM posts p WHERE p.topic_id = t.id) + 1,
                t.last_post_at
         FROM topics t JOIN users u ON u.id = t.user_id
         WHERE t.category_id = ?1
         ORDER BY t.last_post_at DESC LIMIT 100",
    )?;
    let rows = stmt.query_map([category_id], |row| {
        Ok(Topic {
            id: row.get(0)?,
            title: row.get(1)?,
            username: row.get(2)?,
            posts: row.get(3)?,
            last_post_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

/// Crée un sujet (le corps initial est le premier message).
pub fn create_topic(
    conn: &Connection,
    category_id: i64,
    user_id: i64,
    title: &str,
    body: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO topics(category_id, user_id, title, body) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![category_id, user_id, title, body],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Titre + auteur d'un sujet.
pub fn topic_header(conn: &Connection, topic_id: i64) -> Result<Option<(String, String)>> {
    conn.query_row(
        "SELECT t.title, u.username FROM topics t JOIN users u ON u.id = t.user_id WHERE t.id = ?1",
        [topic_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

/// Corps initial d'un sujet.
pub fn topic_body(conn: &Connection, topic_id: i64) -> Result<Option<String>> {
    conn.query_row("SELECT body FROM topics WHERE id = ?1", [topic_id], |r| r.get(0))
        .optional()
}

/// Réponses d'un sujet.
pub fn list_posts(conn: &Connection, topic_id: i64) -> Result<Vec<Post>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, u.username, p.body, p.created_at
         FROM posts p JOIN users u ON u.id = p.user_id
         WHERE p.topic_id = ?1 ORDER BY p.id ASC",
    )?;
    let rows = stmt.query_map([topic_id], |row| {
        Ok(Post {
            id: row.get(0)?,
            username: row.get(1)?,
            body: row.get(2)?,
            created_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Ajoute une réponse et met à jour l'horodatage du sujet.
pub fn create_post(conn: &Connection, topic_id: i64, user_id: i64, body: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO posts(topic_id, user_id, body) VALUES (?1, ?2, ?3)",
        rusqlite::params![topic_id, user_id, body],
    )?;
    conn.execute(
        "UPDATE topics SET last_post_at = CURRENT_TIMESTAMP WHERE id = ?1",
        [topic_id],
    )?;
    Ok(conn.last_insert_rowid())
}

// ---------------------------------------------------------------------------
// Avis sur le logiciel
// ---------------------------------------------------------------------------

/// Seuil (nombre d'utilisateurs) avant d'ouvrir les avis publics.
pub const REVIEWS_MIN_USERS: i64 = 10;

#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub id: i64,
    pub username: String,
    pub rating: i64,
    pub comment: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RatingSummary {
    pub open: bool,
    pub users: i64,
    pub min_users: i64,
    pub count: i64,
    pub average: f64,
}

/// Un avis par utilisateur (création ou mise à jour).
pub fn upsert_review(conn: &Connection, user_id: i64, rating: i64, comment: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO reviews(user_id, rating, comment) VALUES (?1, ?2, ?3)
         ON CONFLICT(user_id) DO UPDATE SET
            rating = excluded.rating,
            comment = excluded.comment,
            updated_at = CURRENT_TIMESTAMP",
        rusqlite::params![user_id, rating.clamp(1, 5), comment],
    )?;
    Ok(())
}

/// Liste des avis.
pub fn list_reviews(conn: &Connection) -> Result<Vec<Review>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, u.username, r.rating, r.comment, r.created_at
         FROM reviews r JOIN users u ON u.id = r.user_id
         ORDER BY r.id DESC LIMIT 200",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Review {
            id: row.get(0)?,
            username: row.get(1)?,
            rating: row.get(2)?,
            comment: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

/// Résumé des notes + état d'ouverture (réservé aux premiers utilisateurs).
pub fn rating_summary(conn: &Connection) -> Result<RatingSummary> {
    let users = count_users(conn)?;
    let (count, average): (i64, f64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(AVG(rating), 0.0) FROM reviews",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(RatingSummary {
        open: users >= REVIEWS_MIN_USERS,
        users,
        min_users: REVIEWS_MIN_USERS,
        count,
        average,
    })
}

// ---------------------------------------------------------------------------
// Partenaires (publicité de serveur Discord, payante et modérée)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Ad {
    pub id: i64,
    pub title: String,
    pub invite_url: String,
    pub description: Option<String>,
    pub status: String,
    pub created_at: Option<String>,
}

/// Dépôt d'une annonce par un membre (statut « en attente »).
pub fn create_ad(
    conn: &Connection,
    owner_id: i64,
    title: &str,
    invite_url: &str,
    description: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO ads(owner_id, title, invite_url, description) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![owner_id, title, invite_url, description],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Annonces selon leur statut (`active`, `pending`, …).
pub fn list_ads(conn: &Connection, status: &str) -> Result<Vec<Ad>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, invite_url, description, status, created_at
         FROM ads WHERE status = ?1 ORDER BY id DESC LIMIT 100",
    )?;
    let rows = stmt.query_map([status], |row| {
        Ok(Ad {
            id: row.get(0)?,
            title: row.get(1)?,
            invite_url: row.get(2)?,
            description: row.get(3)?,
            status: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

/// Change le statut d'une annonce (réservé à l'administrateur).
pub fn set_ad_status(conn: &Connection, ad_id: i64, status: &str, paid: bool) -> Result<()> {
    conn.execute(
        "UPDATE ads SET status = ?2, paid = ?3 WHERE id = ?1",
        rusqlite::params![ad_id, status, paid as i64],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Statistiques
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct HubStats {
    pub users: i64,
    pub rooms: i64,
    pub messages: i64,
    pub topics: i64,
    pub posts: i64,
    pub reviews: i64,
    pub active_ads: i64,
}

/// Statistiques du hub (aucune donnée personnelle).
pub fn stats(conn: &Connection) -> Result<HubStats> {
    let one = |sql: &str| -> Result<i64> { conn.query_row(sql, [], |r| r.get(0)) };
    Ok(HubStats {
        users: count_users(conn)?,
        rooms: one("SELECT COUNT(*) FROM rooms")?,
        messages: one("SELECT COUNT(*) FROM messages")?,
        topics: one("SELECT COUNT(*) FROM topics")?,
        posts: one("SELECT COUNT(*) FROM posts")?,
        reviews: one("SELECT COUNT(*) FROM reviews")?,
        active_ads: one("SELECT COUNT(*) FROM ads WHERE status = 'active'")?,
    })
}
