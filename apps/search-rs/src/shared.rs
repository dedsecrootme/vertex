use std::time::Duration;

use serde::Deserialize;

use crate::{PageView, SearchResult, network};

/// Public index client. No admin credential or personal database is transmitted.
#[derive(Clone)]
pub struct SharedIndex {
    origin: String,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct SearchReply {
    results: Vec<SearchResult>,
}

#[derive(Deserialize, serde::Serialize)]
pub struct SharedStats {
    pub pages: i64,
    pub domains: i64,
}

impl SharedIndex {
    pub fn new(origin: &str) -> Result<Self, String> {
        let url = network::validate_url(origin)?;
        if url.scheme() != "https"
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("VERTEX_SHARED_API doit etre une origine HTTPS publique".into());
        }
        Ok(Self {
            origin: url.origin().ascii_serialization(),
            client: network::api_client(Duration::from_secs(20), "VertexDesktop/0.13")
                .map_err(|e| e.to_string())?,
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, String> {
        let response = self
            .client
            .get(format!("{}{path}", self.origin))
            .query(query)
            .send()
            .await
            .map_err(|_| "Index partage injoignable".to_string())?
            .error_for_status()
            .map_err(|_| "Index partage indisponible".to_string())?;
        let bytes = network::read_limited_async(response, 8_000_000).await?;
        serde_json::from_slice(&bytes).map_err(|_| "Reponse de l'index invalide".into())
    }

    pub async fn search(&self, query: &str, offset: usize) -> Result<Vec<SearchResult>, String> {
        if query.len() > 256 || offset > 30_000 {
            return Err("Recherche excessive".into());
        }
        Ok(self
            .get::<SearchReply>(
                "/api/search",
                &[("q", query), ("offset", &offset.to_string())],
            )
            .await?
            .results)
    }

    pub async fn page(&self, url: &str) -> Result<Option<PageView>, String> {
        let response = self
            .client
            .get(format!("{}/api/page", self.origin))
            .query(&[("url", url)])
            .send()
            .await
            .map_err(|_| "Index partage injoignable".to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response
            .error_for_status()
            .map_err(|_| "Lecture indisponible".to_string())?;
        let bytes = network::read_limited_async(response, 8_000_000).await?;
        let mut page: PageView =
            serde_json::from_slice(&bytes).map_err(|_| "Page invalide".to_string())?;
        let base = network::validate_url(&page.url)?;
        page.content_html = crawler_rs::parser::sanitize_html(&page.content_html, &base);
        page.bookmarked = false;
        Ok(Some(page))
    }

    pub async fn stats(&self) -> Result<SharedStats, String> {
        self.get("/api/stats", &[]).await
    }

    pub async fn discover(&self, url: &str) -> Result<String, String> {
        #[derive(serde::Serialize)]
        struct Input<'a> {
            url: &'a str,
        }
        #[derive(Deserialize)]
        struct Reply {
            status: String,
        }
        let response = self
            .client
            .post(format!("{}/api/discover", self.origin))
            .json(&Input { url })
            .send()
            .await
            .map_err(|_| "Index partage injoignable".to_string())?
            .error_for_status()
            .map_err(|_| "Demande d'indexation indisponible".to_string())?;
        let bytes = network::read_limited_async(response, 4096).await?;
        serde_json::from_slice::<Reply>(&bytes)
            .map(|r| r.status)
            .map_err(|_| "Reponse invalide".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn central_index_must_be_a_public_https_origin() {
        for origin in [
            "http://example.com",
            "https://127.0.0.1",
            "https://example.com/path",
            "https://user:secret@example.com",
        ] {
            assert!(SharedIndex::new(origin).is_err());
        }
        assert!(SharedIndex::new("https://index.example.com").is_ok());
    }
}
