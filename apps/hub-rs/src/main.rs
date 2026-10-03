//! vx-hub — service communautaire Vertex : chat en direct, forums, avis sur le
//! logiciel et partenaires (publicité Discord payante et modérée).
//!
//! Serveur HTTP **écrit à la main**, aucune dépendance réseau, aléa et
//! dérivation de mot de passe maison (voir `auth.rs`), stockage SQLite.

// Le serveur expose volontairement une surface d'API complète (helpers et
// routes destinés aux évolutions) : on n'avertit pas pour le code encore
// inutilisé par le routeur actuel.
#![allow(dead_code)]

mod auth;
mod http;
mod store;

use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use http::{read_request, write_response, Request, Response};
use store::User;

/// Interface de la communauté (embarquée dans le binaire).
const COMMUNITY_HTML: &str = include_str!("../public/index.html");

const SESSION_COOKIE: &str = "vx_session";
const SESSION_DAYS: i64 = 30;

type Db = Arc<Mutex<rusqlite::Connection>>;

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(8787);

    let db_path = store::default_path();
    let conn = match store::init(&db_path) {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("hub : impossible d'ouvrir la base : {error}");
            std::process::exit(1);
        }
    };

    // Création/promotion facultative d'un administrateur via l'environnement.
    if let (Ok(raw_username), Ok(password)) = (
        std::env::var("VX_ADMIN_USER"),
        std::env::var("VX_ADMIN_PASS"),
    ) {
        // On coupe les espaces parasites (notamment l'espace que cmd inclut
        // après `set VAR=value && …`).
        let username = raw_username.trim().to_string();
        if !username.is_empty() && password.len() >= 12 {
            match store::find_user(&conn, &username) {
                Ok(Some((user, _))) => {
                    conn.execute("UPDATE users SET role = 'admin' WHERE id = ?1", [user.id])
                        .ok();
                    println!("hub : « {username} » promu administrateur.");
                }
                Ok(None) => {
                    let hash = auth::hash_password(&password);
                    if let Ok(id) = store::create_user(&conn, &username, &hash) {
                        conn.execute("UPDATE users SET role = 'admin' WHERE id = ?1", [id])
                            .ok();
                        println!("hub : administrateur « {username} » créé.");
                    }
                }
                Err(_) => {}
            }
        }
    }

    let db: Db = Arc::new(Mutex::new(conn));
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("hub : port {port} indisponible : {error}");
            std::process::exit(1);
        }
    };

    println!("Vertex Hub (vx-hub) sur http://127.0.0.1:{port}");
    println!("Base : {}", db_path.display());
    println!("Aucun log de requête n'est conservé.");

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let db = Arc::clone(&db);
        std::thread::spawn(move || {
            if let Some(request) = read_request(&mut stream) {
                let response = handle(&db, &request);
                write_response(&mut stream, &response);
            }
        });
    }
}

/// Utilisateur courant à partir du cookie de session.
fn current_user(conn: &rusqlite::Connection, request: &Request) -> Option<User> {
    let token = request.cookie(SESSION_COOKIE)?;
    store::user_for_token(conn, token).ok().flatten()
}

/// Extrait une chaîne d'un corps JSON.
fn field(body: &serde_json::Value, key: &str) -> String {
    body.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Vrai si le pseudo est acceptable (3–32 caractères alphanumériques, `_`, `-`).
fn valid_username(name: &str) -> bool {
    let len = name.chars().count();
    (3..=32).contains(&len)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Routage : découpe le chemin et délègue selon la méthode.
fn handle(db: &Db, request: &Request) -> Response {
    let segments: Vec<&str> = request
        .path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    match request.method.as_str() {
        "GET" => handle_get(db, request, &segments),
        "POST" => handle_post(db, request, &segments),
        _ => Response::json_error(404, "méthode non supportée"),
    }
}

fn db_error(error: impl std::fmt::Display) -> Response {
    Response::json_error(500, &error.to_string())
}

fn handle_get(db: &Db, request: &Request, segments: &[&str]) -> Response {
    let conn = match db.lock() {
        Ok(guard) => guard,
        Err(_) => return Response::json_error(500, "verrou de base indisponible"),
    };

    match segments {
        [] => Response::html(COMMUNITY_HTML.to_string()),

        ["api", "stats"] => match store::stats(&conn) {
            Ok(stats) => Response::json(serde_json::json!(stats)),
            Err(error) => db_error(error),
        },

        ["api", "me"] => match current_user(&conn, request) {
            Some(user) => Response::json(serde_json::json!({ "user": user })),
            None => Response::json(serde_json::json!({ "user": serde_json::Value::Null })),
        },

        ["api", "rooms"] => match store::list_rooms(&conn) {
            Ok(rooms) => Response::json(serde_json::json!({ "rooms": rooms })),
            Err(error) => db_error(error),
        },

        ["api", "rooms", slug, "messages"] => {
            let since: i64 = request.param("since").and_then(|v| v.parse().ok()).unwrap_or(0);
            match store::room_by_slug(&conn, slug) {
                Ok(Some(room)) => match store::list_messages(&conn, room.id, since, 200) {
                    Ok(messages) => {
                        Response::json(serde_json::json!({ "room": room, "messages": messages }))
                    }
                    Err(error) => db_error(error),
                },
                Ok(None) => Response::json_error(404, "salon inconnu"),
                Err(error) => db_error(error),
            }
        }

        ["api", "forum"] => match store::list_categories(&conn) {
            Ok(categories) => Response::json(serde_json::json!({ "categories": categories })),
            Err(error) => db_error(error),
        },

        ["api", "forum", slug, "topics"] => match store::category_id(&conn, slug) {
            Ok(Some(id)) => match store::list_topics(&conn, id) {
                Ok(topics) => Response::json(serde_json::json!({ "topics": topics })),
                Err(error) => db_error(error),
            },
            Ok(None) => Response::json_error(404, "catégorie inconnue"),
            Err(error) => db_error(error),
        },

        ["api", "forum", "topic", id] => match id.parse::<i64>() {
            Ok(topic_id) => match store::topic_header(&conn, topic_id) {
                Ok(Some((title, author))) => {
                    let body = store::topic_body(&conn, topic_id).unwrap_or_default();
                    let posts = store::list_posts(&conn, topic_id).unwrap_or_default();
                    Response::json(serde_json::json!({
                        "topic": { "id": topic_id, "title": title, "username": author, "body": body },
                        "posts": posts
                    }))
                }
                Ok(None) => Response::json_error(404, "sujet inconnu"),
                Err(error) => db_error(error),
            },
            Err(_) => Response::json_error(400, "identifiant invalide"),
        },

        ["api", "reviews"] => match (store::rating_summary(&conn), store::list_reviews(&conn)) {
            (Ok(summary), Ok(reviews)) => {
                Response::json(serde_json::json!({ "summary": summary, "reviews": reviews }))
            }
            (Err(error), _) | (_, Err(error)) => db_error(error),
        },

        ["api", "ads"] => match store::list_ads(&conn, "active") {
            Ok(ads) => Response::json(serde_json::json!({ "ads": ads })),
            Err(error) => db_error(error),
        },

        _ => Response::json_error(404, "route inconnue"),
    }
}

/// Longueur minimale d'un mot de passe (le coffre en génère de bien plus longs).
const MIN_PASSWORD: usize = 10;

fn handle_post(db: &Db, request: &Request, segments: &[&str]) -> Response {
    let conn = match db.lock() {
        Ok(guard) => guard,
        Err(_) => return Response::json_error(500, "verrou de base indisponible"),
    };

    let body = request.json().unwrap_or(serde_json::Value::Null);

    match segments {
        // --- Comptes ------------------------------------------------------
        ["api", "register"] => {
            let username = field(&body, "username");
            let password = field(&body, "password");
            if !valid_username(&username) {
                return Response::json_error(
                    400,
                    "pseudo invalide (3–32 caractères : lettres, chiffres, _ ou -)",
                );
            }
            if password.chars().count() < MIN_PASSWORD {
                return Response::json_error(400, "mot de passe trop court (10 caractères minimum)");
            }
            if store::find_user(&conn, &username).ok().flatten().is_some() {
                return Response::json_error(409, "ce pseudo est déjà pris");
            }
            let hash = auth::hash_password(&password);
            match store::create_user(&conn, &username, &hash) {
                Ok(id) => {
                    let token = auth::new_session_token();
                    store::create_session(&conn, id, &token, SESSION_DAYS).ok();
                    Response::json(serde_json::json!({ "ok": true, "username": username }))
                        .cookie(SESSION_COOKIE, &token, SESSION_DAYS * 86_400)
                }
                Err(error) => db_error(error),
            }
        }

        ["api", "login"] => {
            let username = field(&body, "username");
            let password = field(&body, "password");
            match store::find_user(&conn, &username) {
                Ok(Some((user, hash))) if auth::verify_password(&password, &hash) => {
                    let token = auth::new_session_token();
                    store::create_session(&conn, user.id, &token, SESSION_DAYS).ok();
                    Response::json(serde_json::json!({ "ok": true, "user": user }))
                        .cookie(SESSION_COOKIE, &token, SESSION_DAYS * 86_400)
                }
                // Réponse identique que le compte existe ou non (pas d'énumération).
                _ => Response::json_error(401, "identifiants incorrects"),
            }
        }

        ["api", "logout"] => {
            if let Some(token) = request.cookie(SESSION_COOKIE) {
                store::delete_session(&conn, token).ok();
            }
            Response::json(serde_json::json!({ "ok": true })).cookie(SESSION_COOKIE, "", 0)
        }

        // --- Chat ---------------------------------------------------------
        ["api", "rooms", slug, "messages"] => {
            let Some(user) = current_user(&conn, request) else {
                return Response::json_error(401, "authentification requise");
            };
            let text = field(&body, "body");
            if text.is_empty() || text.chars().count() > 2000 {
                return Response::json_error(400, "message vide ou trop long");
            }
            match store::room_by_slug(&conn, slug) {
                Ok(Some(room)) => match store::post_message(&conn, room.id, user.id, &text) {
                    Ok(id) => Response::json(serde_json::json!({ "ok": true, "id": id })),
                    Err(error) => db_error(error),
                },
                Ok(None) => Response::json_error(404, "salon inconnu"),
                Err(error) => db_error(error),
            }
        }

        // --- Forums -------------------------------------------------------
        ["api", "forum", slug, "topics"] => {
            let Some(user) = current_user(&conn, request) else {
                return Response::json_error(401, "authentification requise");
            };
            let title = field(&body, "title");
            let text = field(&body, "body");
            if title.chars().count() < 3 || text.is_empty() {
                return Response::json_error(400, "titre ou message manquant");
            }
            match store::category_id(&conn, slug) {
                Ok(Some(id)) => match store::create_topic(&conn, id, user.id, &title, &text) {
                    Ok(topic_id) => {
                        Response::json(serde_json::json!({ "ok": true, "id": topic_id }))
                    }
                    Err(error) => db_error(error),
                },
                Ok(None) => Response::json_error(404, "catégorie inconnue"),
                Err(error) => db_error(error),
            }
        }

        ["api", "forum", "topic", id, "reply"] => {
            let Some(user) = current_user(&conn, request) else {
                return Response::json_error(401, "authentification requise");
            };
            let Ok(topic_id) = id.parse::<i64>() else {
                return Response::json_error(400, "identifiant invalide");
            };
            let text = field(&body, "body");
            if text.is_empty() {
                return Response::json_error(400, "réponse vide");
            }
            match store::create_post(&conn, topic_id, user.id, &text) {
                Ok(post_id) => Response::json(serde_json::json!({ "ok": true, "id": post_id })),
                Err(error) => db_error(error),
            }
        }

        // --- Avis (ouverts avec les premiers utilisateurs) ----------------
        ["api", "reviews"] => {
            let Some(user) = current_user(&conn, request) else {
                return Response::json_error(401, "authentification requise");
            };
            let summary = match store::rating_summary(&conn) {
                Ok(summary) => summary,
                Err(error) => return db_error(error),
            };
            if !summary.open {
                return Response::json_error(
                    403,
                    "les avis s'ouvriront avec les premiers utilisateurs",
                );
            }
            let rating = body.get("rating").and_then(|v| v.as_i64()).unwrap_or(0);
            if !(1..=5).contains(&rating) {
                return Response::json_error(400, "note attendue entre 1 et 5");
            }
            let comment = field(&body, "comment");
            match store::upsert_review(&conn, user.id, rating, &comment) {
                Ok(()) => Response::json(serde_json::json!({ "ok": true })),
                Err(error) => db_error(error),
            }
        }

        // --- Partenaires (publicité Discord) ------------------------------
        ["api", "ads"] => {
            let Some(user) = current_user(&conn, request) else {
                return Response::json_error(401, "authentification requise");
            };
            let title = field(&body, "title");
            let invite = field(&body, "invite_url");
            let description = field(&body, "description");
            if title.is_empty() || !invite.starts_with("https://discord.") {
                return Response::json_error(
                    400,
                    "titre requis et lien Discord (https://discord.gg/…) requis",
                );
            }
            match store::create_ad(&conn, user.id, &title, &invite, &description) {
                Ok(id) => Response::json(serde_json::json!({
                    "ok": true, "id": id, "status": "pending",
                    "message": "Annonce reçue : publication après vérification et paiement."
                })),
                Err(error) => db_error(error),
            }
        }

        ["api", "ads", id, "moderate"] => {
            let Some(user) = current_user(&conn, request) else {
                return Response::json_error(401, "authentification requise");
            };
            if user.role != "admin" {
                return Response::json_error(403, "réservé à l'administrateur");
            }
            let Ok(ad_id) = id.parse::<i64>() else {
                return Response::json_error(400, "identifiant invalide");
            };
            let status = field(&body, "status");
            if !matches!(status.as_str(), "active" | "rejected" | "pending" | "expired") {
                return Response::json_error(400, "statut invalide");
            }
            let paid = body.get("paid").and_then(|v| v.as_bool()).unwrap_or(false);
            match store::set_ad_status(&conn, ad_id, &status, paid) {
                Ok(()) => Response::json(serde_json::json!({ "ok": true, "status": status })),
                Err(error) => db_error(error),
            }
        }

        _ => Response::json_error(404, "route inconnue"),
    }
}
