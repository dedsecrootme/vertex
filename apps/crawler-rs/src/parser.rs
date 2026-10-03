//! Extraction et nettoyage du HTML : titre, description, langue, favicon,
//! liens complets, images, texte indexable et HTML assaini pour la visionneuse.

use regex::Regex;
use scraper::{Html, Selector};
use std::sync::LazyLock;
use url::Url;

use crate::models::{PageImage, PageLink};

/// Blocs retirés du texte indexable (métadonnées, navigation, scripts, pub…).
static JUNK_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    const PATTERNS: &[&str] = &[
        r"(?is)<script\b[^>]*>.*?</script>",
        r"(?is)<style\b[^>]*>.*?</style>",
        r"(?is)<noscript\b[^>]*>.*?</noscript>",
        r"(?is)<nav\b[^>]*>.*?</nav>",
        r"(?is)<header\b[^>]*>.*?</header>",
        r"(?is)<footer\b[^>]*>.*?</footer>",
        r"(?is)<aside\b[^>]*>.*?</aside>",
        r"(?is)<form\b[^>]*>.*?</form>",
        r"(?is)<iframe\b[^>]*>.*?</iframe>",
        r"(?is)<svg\b[^>]*>.*?</svg>",
        r"(?is)<template\b[^>]*>.*?</template>",
        r"(?is)<video\b[^>]*>.*?</video>",
        r"(?is)<audio\b[^>]*>.*?</audio>",
        r"(?is)<picture\b[^>]*>.*?</picture>",
        r"(?is)<object\b[^>]*>.*?</object>",
        r"(?is)<canvas\b[^>]*>.*?</canvas>",
        r"(?is)<embed\b[^>]*>.*?</embed>",
        r"(?is)<map\b[^>]*>.*?</map>",
        r"(?is)<head\b[^>]*>.*?</head>",
        r"(?is)<title\b[^>]*>.*?</title>",
    ];
    PATTERNS
        .iter()
        .map(|p| Regex::new(p).expect("Regex invalide"))
        .collect()
});

/// Éléments/jetons publicitaires ou de suivi, retirés du HTML de la visionneuse.
static AD_TRACKER_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    const PATTERNS: &[&str] = &[
        r"(?is)<ins\b[^>]*>.*?</ins>",
        r"(?is)<script\b[^>]*>.*?</script>",
        r"(?is)<style\b[^>]*>.*?</style>",
        r"(?is)<noscript\b[^>]*>.*?</noscript>",
        r"(?is)<iframe\b[^>]*>.*?</iframe>",
        r"(?is)<object\b[^>]*>.*?</object>",
        r"(?is)<embed\b[^>]*>.*?</embed>",
        r"(?is)<form\b[^>]*>.*?</form>",
        r"(?is)<svg\b[^>]*>.*?</svg>",
        r"(?is)<canvas\b[^>]*>.*?</canvas>",
        r"(?is)<template\b[^>]*>.*?</template>",
        r"(?is)<link\b[^>]*>",
        r"(?is)<base\b[^>]*>",
        r"(?is)<meta\b[^>]*>",
        r#"(?is)<(?:div|section|aside|span|li)\b[^>]*(?:class|id)\s*=\s*"[^"]*(?:advert|adsense|\bads?\b|banner|sponsor|promo|popup|modal|paywall|cookie|consent|newsletter|subscribe|social-share|share-buttons|taboola|outbrain|doubleclick|analytics|tracker|gpt-ad)[^"]*"[^>]*>.*?</(?:div|section|aside|span|li)>"#,
    ];
    PATTERNS
        .iter()
        .map(|p| Regex::new(p).expect("Regex invalide"))
        .collect()
});

static RE_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<!--.*?-->").expect("comment regex"));
static RE_EVENT_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\son[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)"#).expect("event"));
static RE_STYLE_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\sstyle\s*=\s*("[^"]*"|'[^']*')"#).expect("style"));
static RE_IMG_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<img\b[^>]*>").expect("img"));
static RE_IMG_SRC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\ssrc\s*=\s*"([^"]*)""#).expect("src dq"));
static RE_IMG_SRC_SQ: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\ssrc\s*=\s*'([^']*)'"#).expect("src sq"));
static RE_SRCSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\ssrcset\s*=\s*("[^"]*"|'[^']*')"#).expect("srcset"));
static RE_ANCHOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<a\b([^>]*)>"#).expect("a"));
static RE_HREF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\shref\s*=\s*"([^"]*)""#).expect("href"));
static RE_HREF_SQ: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\shref\s*=\s*'([^']*)'"#).expect("href sq"));
static RE_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("ws"));

/// Paramètres de suivi retirés des URL.
const TRACKING_PARAMS: &[&str] = &[
    "utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content", "utm_id", "utm_name",
    "utm_reader", "utm_referrer", "fbclid", "gclid", "gclsrc", "dclid", "msclkid", "yclid",
    "mc_cid", "mc_eid", "_ga", "_gl", "igshid", "ref_src", "ref_url", "spm", "scm", "affid",
    "aff_id", "affiliate", "partner", "s_kwcid", "wickedid",
];

/// Supprime les paramètres de suivi/affiliation d'une URL.
pub fn strip_tracking(url: &mut Url) {
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| {
            let key = k.to_ascii_lowercase();
            !TRACKING_PARAMS.contains(&key.as_str()) && !key.starts_with("utm_")
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.set_query(None);
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(pairs);
    }
}

/// Résout une URL relative en URL absolue, en retirant les paramètres de suivi.
pub fn absolutize(base: &Url, link: &str) -> Option<String> {
    let link = link.trim();
    if link.is_empty() || link.starts_with('#') {
        return None;
    }
    let lowered = link.to_ascii_lowercase();
    if lowered.starts_with("javascript:")
        || lowered.starts_with("mailto:")
        || lowered.starts_with("tel:")
        || lowered.starts_with("data:")
        || lowered.starts_with("about:")
    {
        return None;
    }
    let mut url = base.join(link).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    strip_tracking(&mut url);
    Some(url.to_string())
}

/// Extrait le titre d'une page HTML.
pub fn extract_title(html: &str) -> String {
    title_from(&Html::parse_document(html))
}

/// Extrait le titre depuis un document déjà analysé.
fn title_from(document: &Html) -> String {
    let selector = Selector::parse("title").expect("sélecteur title");
    if let Some(el) = document.select(&selector).next() {
        let title = collapse_whitespace(&el.text().collect::<String>());
        if !title.is_empty() {
            return title;
        }
    }
    let og = Selector::parse("meta[property='og:title']").expect("og:title");
    document
        .select(&og)
        .next()
        .and_then(|e| e.value().attr("content"))
        .map(collapse_whitespace)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Sans titre".to_string())
}

/// Extrait la description (meta description ou og:description).
pub fn extract_description(html: &str) -> Option<String> {
    description_from(&Html::parse_document(html))
}

fn description_from(document: &Html) -> Option<String> {
    for sel in ["meta[name='description']", "meta[property='og:description']"] {
        if let Ok(selector) = Selector::parse(sel)
            && let Some(el) = document.select(&selector).next()
            && let Some(content) = el.value().attr("content")
        {
            let content = collapse_whitespace(content);
            if !content.is_empty() {
                return Some(content);
            }
        }
    }
    None
}

/// Extrait la langue déclarée par la page (`<html lang>`).
pub fn extract_language(html: &str) -> Option<String> {
    language_from(&Html::parse_document(html))
}

fn language_from(document: &Html) -> Option<String> {
    let selector = Selector::parse("html").expect("html");
    document
        .select(&selector)
        .next()
        .and_then(|e| e.value().attr("lang"))
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
}

/// Extrait l'URL du favicon déclaré par la page.
pub fn extract_favicon(html: &str, base: &Url) -> Option<String> {
    favicon_from(&Html::parse_document(html), base)
}

fn favicon_from(document: &Html, base: &Url) -> Option<String> {
    let selector = Selector::parse("link[rel*='icon']").expect("link icon");
    for el in document.select(&selector) {
        if let Some(href) = el.value().attr("href")
            && let Some(abs) = absolutize(base, href)
        {
            return Some(abs);
        }
    }
    None
}

/// Extrait les liens complets d'une page (URL absolue, ancre, externe, sûreté).
pub fn extract_links(html: &str, base: &Url) -> Vec<PageLink> {
    links_from(&Html::parse_document(html), base)
}

fn links_from(document: &Html, base: &Url) -> Vec<PageLink> {
    let selector = Selector::parse("a[href]").expect("sélecteur a");
    let base_host = base.host_str().unwrap_or("").to_string();
    let mut links = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for el in document.select(&selector) {
        let Some(href) = el.value().attr("href") else {
            continue;
        };
        let Some(url) = absolutize(base, href) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let is_external = Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .map(|h| h != base_host)
            .unwrap_or(false);
        let anchor = collapse_whitespace(&el.text().collect::<String>());
        let anchor = if anchor.is_empty() { None } else { Some(anchor) };
        let safety = crate::safety::check_url(&url).verdict.as_str().to_string();
        links.push(PageLink {
            url,
            anchor,
            is_external,
            safety,
        });
    }
    links
}

/// Extrait les images d'une page (URL absolue + texte alternatif).
pub fn extract_images(html: &str, base: &Url) -> Vec<PageImage> {
    images_from(&Html::parse_document(html), base)
}

fn images_from(document: &Html, base: &Url) -> Vec<PageImage> {
    let selector = Selector::parse("img").expect("sélecteur img");
    let mut images = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for el in document.select(&selector) {
        let src = el
            .value()
            .attr("src")
            .or_else(|| el.value().attr("data-src"))
            .or_else(|| el.value().attr("data-original"));
        let Some(src) = src else { continue };
        let Some(url) = absolutize(base, src) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let alt = el
            .value()
            .attr("alt")
            .map(collapse_whitespace)
            .filter(|a| !a.is_empty());
        images.push(PageImage { url, alt });
    }
    images
}

/// Retire les blocs hors contenu puis renvoie le HTML restant.
fn remove_junk(html: &str) -> String {
    let mut out = html.to_string();
    for re in JUNK_PATTERNS.iter() {
        out = re.replace_all(&out, " ").to_string();
    }
    out
}

/// Réduit tout espace (retours à la ligne, tabulations, espaces répétés) en un seul.
fn collapse_whitespace(text: &str) -> String {
    RE_WHITESPACE.replace_all(text, " ").trim().to_string()
}

/// Résultat d'une analyse complète de page (un seul parse du DOM).
pub struct Parsed {
    pub title: String,
    pub description: Option<String>,
    pub language: Option<String>,
    pub favicon: Option<String>,
    pub links: Vec<PageLink>,
    pub images: Vec<PageImage>,
    pub text: String,
    pub sanitized_html: String,
}

/// Analyse une page en ne parsant le DOM qu'**une seule fois** pour toutes les
/// métadonnées (titre, description, langue, favicon, liens, images) — les deux
/// passes restantes (texte indexable et HTML assaini) sont des passes sur le
/// texte, bien moins coûteuses que des parses répétés.
pub fn analyze(html: &str, base: &Url) -> Parsed {
    let document = Html::parse_document(html);
    Parsed {
        title: title_from(&document),
        description: description_from(&document),
        language: language_from(&document),
        favicon: favicon_from(&document, base),
        links: links_from(&document, base),
        images: images_from(&document, base),
        text: extract_text(html),
        sanitized_html: sanitize_html(html, base),
    }
}

/// Extrait le texte visible et propre de la page (sans navigation ni scripts).
pub fn extract_text(html: &str) -> String {
    let sanitized = remove_junk(html);
    let document = Html::parse_document(&sanitized);
    let selector = Selector::parse("body").expect("sélecteur body");
    let mut text = String::new();
    for element in document.select(&selector) {
        text.push_str(&element.text().collect::<Vec<_>>().join(" "));
    }
    collapse_whitespace(&text)
}

static RE_IMG_ALT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\salt\s*=\s*"([^"]*)""#).expect("alt dq"));
static RE_IMG_ALT_SQ: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\salt\s*=\s*'([^']*)'"#).expect("alt sq"));
static RE_DOCTYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<!doctype[^>]*>").expect("doctype"));
static RE_HTML_TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)</?html\b[^>]*>|</?body\b[^>]*>|<head\b[^>]*>.*?</head>").expect("html tags"));

/// Taille maximale du HTML assaini conservé (en octets) pour la visionneuse.
pub const MAX_SANITIZED_HTML: usize = 1_500_000;

fn html_escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn first_capture(tag: &str, dq: &Regex, sq: &Regex) -> Option<String> {
    dq.captures(tag)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .or_else(|| sq.captures(tag).and_then(|c| c.get(1)).map(|m| m.as_str().to_string()))
}

/// Produit un HTML assaini, sans scripts, iframes, trackers, publicités,
/// pixels espions ni paramètres d'affiliation.
///
/// Les liens et images sont réécrits en URL absolues ; les liens externes
/// s'ouvrent avec `rel="noopener noreferrer"` et les images ne transmettent
/// pas de referer.
pub fn sanitize_html(html: &str, base: &Url) -> String {
    let mut out = RE_DOCTYPE.replace_all(html, "").to_string();
    out = RE_COMMENT.replace_all(&out, "").to_string();
    out = RE_HTML_TAGS.replace_all(&out, "").to_string();
    for re in AD_TRACKER_PATTERNS.iter() {
        out = re.replace_all(&out, "").to_string();
    }
    out = RE_EVENT_ATTR.replace_all(&out, "").to_string();
    out = RE_STYLE_ATTR.replace_all(&out, "").to_string();

    out = RE_IMG_TAG
        .replace_all(&out, |caps: &regex::Captures| -> String {
            let tag = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let src = first_capture(tag, &RE_IMG_SRC, &RE_IMG_SRC_SQ);
            let alt = first_capture(tag, &RE_IMG_ALT, &RE_IMG_ALT_SQ).unwrap_or_default();
            match src.and_then(|s| absolutize(base, &s)) {
                Some(url) => format!(
                    "<img src=\"{}\" alt=\"{}\" loading=\"lazy\" referrerpolicy=\"no-referrer\">",
                    html_escape_attr(&url),
                    html_escape_attr(&alt)
                ),
                None => String::new(),
            }
        })
        .to_string();

    out = RE_SRCSET.replace_all(&out, "").to_string();

    out = RE_ANCHOR
        .replace_all(&out, |caps: &regex::Captures| -> String {
            let tag = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let href = first_capture(tag, &RE_HREF, &RE_HREF_SQ);
            match href.and_then(|h| absolutize(base, &h)) {
                Some(url) => format!(
                    "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">",
                    html_escape_attr(&url)
                ),
                None => "<a>".to_string(),
            }
        })
        .to_string();

    if out.len() > MAX_SANITIZED_HTML {
        // Tronque proprement (l'interface ajoute un avis de troncature).
        let mut cut = MAX_SANITIZED_HTML;
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/a/b/").unwrap()
    }

    #[test]
    fn extracts_and_absolutizes_links() {
        let html = r#"<a href="../guide?utm_source=x&id=7">Guide</a><a href="javascript:void(0)">x</a>"#;
        let links = extract_links(html, &base());
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://example.com/a/guide?id=7");
        assert_eq!(links[0].anchor.as_deref(), Some("Guide"));
    }

    #[test]
    fn extracts_images_with_alt() {
        let html = r#"<img src="/pic.png" alt="Chat"><img data-src="https://cdn.x/y.jpg">"#;
        let images = extract_images(html, &base());
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].url, "https://example.com/pic.png");
        assert_eq!(images[0].alt.as_deref(), Some("Chat"));
    }

    #[test]
    fn sanitizer_removes_scripts_and_trackers() {
        let html = r#"<html><head><title>t</title></head><body>
            <script>alert(1)</script>
            <div class="ads-banner">PUB</div>
            <p onclick="steal()">Texte</p>
            <img src="/x.png" srcset="a 2x" referrerpolicy="unsafe-url">
            <a href="/next?utm_source=nl" rel="nofollow">Suite</a>
        </body></html>"#;
        let clean = sanitize_html(html, &base());
        assert!(!clean.contains("<script"));
        assert!(!clean.contains("ads-banner"));
        assert!(!clean.contains("onclick"));
        assert!(!clean.contains("srcset"));
        assert!(clean.contains("https://example.com/x.png"));
        // `/next` est un chemin absolu : il se résout à la racine du domaine.
        assert!(clean.contains("https://example.com/next"));
        assert!(clean.contains("referrerpolicy=\"no-referrer\""));
    }

    #[test]
    fn strip_tracking_removes_affiliate_params() {
        let mut u = Url::parse("https://shop.example/p?affid=42&id=9&fbclid=zzz").unwrap();
        strip_tracking(&mut u);
        let q = u.query().unwrap_or("");
        assert!(q.contains("id=9"));
        assert!(!q.contains("affid"));
        assert!(!q.contains("fbclid"));
    }
}
