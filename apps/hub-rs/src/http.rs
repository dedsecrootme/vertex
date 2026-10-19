//! Serveur HTTP/1.1 minimal écrit à la main — **aucune dépendance réseau**.
//!
//! Gère : ligne de requête, en-têtes, corps `Content-Length`, paramètres de
//! requête, cookies, et écrit la réponse puis ferme la connexion.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// Taille maximale acceptée pour un corps de requête (1 Mo).
const MAX_BODY: usize = 1_048_576;
const MAX_LINE: usize = 8192;
const MAX_HEADERS: usize = 32_768;

fn bounded_line(reader: &mut impl BufRead) -> Option<String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .ok()?;
    if bytes.len() > MAX_LINE || !bytes.ends_with(b"\r\n") {
        return None;
    }
    String::from_utf8(bytes).ok()
}

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn body_string(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }

    pub fn param(&self, key: &str) -> Option<&str> {
        self.query.get(key).map(String::as_str)
    }

    pub fn cookie(&self, key: &str) -> Option<&str> {
        self.cookies.get(key).map(String::as_str)
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl Response {
    fn new(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: content_type.to_string(),
            body,
            headers: Vec::new(),
        }
    }

    pub fn html(body: String) -> Self {
        Response::new(200, "text/html; charset=utf-8", body.into_bytes())
    }

    pub fn json(value: serde_json::Value) -> Self {
        Response::new(
            200,
            "application/json; charset=utf-8",
            value.to_string().into_bytes(),
        )
    }

    pub fn json_error(status: u16, message: &str) -> Self {
        let body = serde_json::json!({ "error": message }).to_string();
        Response::new(status, "application/json; charset=utf-8", body.into_bytes())
    }

    pub fn no_content(status: u16) -> Self {
        Response::new(status, "text/plain; charset=utf-8", Vec::new())
    }

    /// Ajoute un en-tête `Set-Cookie` (session).
    pub fn cookie(mut self, name: &str, value: &str, max_age: i64) -> Self {
        self.headers.push((
            "Set-Cookie".to_string(),
            format!("{name}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}"),
        ));
        self
    }

    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.headers.push((key.to_string(), value.to_string()));
        self
    }
}

/// Décode les séquences `%XX` et `+` d'une valeur d'URL.
fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn parse_query(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(url_decode(k), url_decode(v));
    }
    map
}

/// Lit une requête HTTP complète (en-têtes + corps) depuis la connexion.
pub fn read_request(stream: &mut TcpStream) -> Option<Request> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok()?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(10)))
        .ok()?;
    parse_request(BufReader::new(stream.try_clone().ok()?))
}

fn parse_request(mut reader: impl BufRead) -> Option<Request> {
    let request_line = bounded_line(&mut reader)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    if !matches!(parts.next()?, "HTTP/1.0" | "HTTP/1.1")
        || parts.next().is_some()
        || !target.starts_with('/')
    {
        return None;
    }

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), parse_query(q)),
        None => (target.clone(), HashMap::new()),
    };

    let mut headers = HashMap::new();
    let mut header_bytes = 0;
    loop {
        let line = bounded_line(&mut reader)?;
        header_bytes += line.len();
        if header_bytes > MAX_HEADERS {
            return None;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        let (key, value) = line.split_once(':')?;
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        {
            return None;
        }
        if headers
            .insert(key.to_ascii_lowercase(), value.trim().to_string())
            .is_some()
        {
            return None;
        }
    }

    let mut cookies = HashMap::new();
    if let Some(raw) = headers.get("cookie") {
        for pair in raw.split(';') {
            if let Some((k, v)) = pair.split_once('=') {
                cookies.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }

    if headers.contains_key("transfer-encoding") {
        return None;
    }
    let length: usize = match headers.get("content-length") {
        Some(value) if value.bytes().all(|b| b.is_ascii_digit()) => value.parse().ok()?,
        Some(_) => return None,
        None => 0,
    };
    if length > MAX_BODY {
        return None;
    }

    let mut body = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut body).ok()?;
    }

    Some(Request {
        method,
        path,
        query,
        headers,
        cookies,
        body,
    })
}

/// Écrit la réponse puis termine la connexion.
pub fn write_response(stream: &mut TcpStream, response: &Response) {
    let reason = match response.status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };

    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    );
    for (key, value) in &response.headers {
        head.push_str(&format!("{key}: {value}\r\n"));
    }
    head.push_str("\r\n");

    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&response.body);
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bounded_request_with_query_cookie_and_body() {
        let request = parse_request(&b"POST /api/x?q=caf%C3%A9 HTTP/1.1\r\nCookie: vx_session=abc\r\nContent-Length: 2\r\n\r\n{}"[..]).unwrap();
        assert_eq!(request.param("q"), Some("café"));
        assert_eq!(request.cookie("vx_session"), Some("abc"));
        assert_eq!(request.body, b"{}");
    }

    #[test]
    fn rejects_ambiguous_and_oversized_requests() {
        for headers in [
            "Content-Length: 1048577",
            "Content-Length: nope",
            "Content-Length: 2\r\nContent-Length: 2",
            "Transfer-Encoding: chunked",
        ] {
            let raw = format!("POST / HTTP/1.1\r\n{headers}\r\n\r\n{{}}");
            assert!(parse_request(raw.as_bytes()).is_none());
        }
        let raw = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(MAX_LINE));
        assert!(parse_request(raw.as_bytes()).is_none());
        assert!(parse_request(&b"GET / HTTP/1.1\r\nHost: x\r\n"[..]).is_none());
    }
}
