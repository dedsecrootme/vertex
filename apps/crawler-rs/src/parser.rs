//! Extraction et nettoyage du HTML : titre, description, langue, favicon,
//! liens complets, images, texte indexable et HTML assaini pour la visionneuse.

use regex::Regex;
use scraper::{Html, Selector};
use std::sync::{Condvar, LazyLock, Mutex};
use url::Url;

use crate::models::{PageImage, PageLink};

struct ParseBudget {
    active: Mutex<usize>,
    changed: Condvar,
    limit: usize,
}
static PARSE_BUDGET: LazyLock<ParseBudget> = LazyLock::new(|| ParseBudget {
    active: Mutex::new(0),
    changed: Condvar::new(),
    limit: std::thread::available_parallelism()
        .map(|n| n.get().div_ceil(4))
        .unwrap_or(1)
        .clamp(1, 4),
});
struct ParsePermit(&'static ParseBudget);
impl ParsePermit {
    fn acquire() -> Self {
        let budget = &*PARSE_BUDGET;
        let active = budget.active.lock().unwrap_or_else(|e| e.into_inner());
        let mut active = budget
            .changed
            .wait_while(active, |active| *active >= budget.limit)
            .unwrap_or_else(|e| e.into_inner());
        *active += 1;
        Self(budget)
    }
}
impl Drop for ParsePermit {
    fn drop(&mut self) {
        *self.0.active.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        self.0.changed.notify_one();
    }
}

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

static RE_IMG_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<img\b[^>]*>").expect("img"));
static RE_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("ws"));

/// Paramètres de suivi retirés des URL.
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "utm_name",
    "utm_reader",
    "utm_referrer",
    "fbclid",
    "gclid",
    "gclsrc",
    "dclid",
    "msclkid",
    "yclid",
    "mc_cid",
    "mc_eid",
    "_ga",
    "_gl",
    "igshid",
    "ref_src",
    "ref_url",
    "spm",
    "scm",
    "affid",
    "aff_id",
    "affiliate",
    "partner",
    "s_kwcid",
    "wickedid",
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
    for sel in [
        "meta[name='description']",
        "meta[property='og:description']",
    ] {
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
    for el in document.select(&selector).take(2000) {
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
        let anchor = if anchor.is_empty() {
            None
        } else {
            Some(anchor)
        };
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

    for el in document.select(&selector).take(500) {
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
    // Network concurrency must not become 64 simultaneous, memory-heavy DOM trees.
    let _permit = ParsePermit::acquire();
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

/// Maximum sanitized HTML stored for the reader.
pub const MAX_SANITIZED_HTML: usize = 1_500_000;

fn html_escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Parses untrusted HTML with an allowlist. Images stay inert until proxied.
pub fn sanitize_html(html: &str, base: &Url) -> String {
    use std::borrow::Cow;
    use std::collections::HashSet;
    let tags: HashSet<&str> = [
        "a",
        "abbr",
        "article",
        "b",
        "blockquote",
        "br",
        "caption",
        "code",
        "dd",
        "del",
        "details",
        "div",
        "dl",
        "dt",
        "em",
        "figcaption",
        "figure",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "i",
        "img",
        "kbd",
        "li",
        "main",
        "ol",
        "p",
        "pre",
        "s",
        "section",
        "small",
        "span",
        "strong",
        "sub",
        "summary",
        "sup",
        "table",
        "tbody",
        "td",
        "th",
        "thead",
        "tr",
        "ul",
    ]
    .into_iter()
    .collect();
    let mut filtered = html.to_string();
    for re in AD_TRACKER_PATTERNS.iter() {
        filtered = re.replace_all(&filtered, "").into_owned();
    }
    let mut builder = ammonia::Builder::default();
    let rewrite_base = base.clone();
    builder
        .tags(tags)
        .generic_attributes(HashSet::new())
        .add_tag_attributes("img", &["data-src"])
        .add_clean_content_tags(&[
            "iframe", "object", "embed", "svg", "math", "form", "template", "video", "audio",
            "canvas",
        ])
        .link_rel(Some("noopener noreferrer"))
        .set_tag_attribute_value("a", "target", "_blank")
        .url_schemes(["http", "https"].into_iter().collect())
        .url_relative(ammonia::UrlRelative::RewriteWithBase(base.clone()))
        .attribute_filter(move |_, attr, value| {
            if matches!(attr, "href" | "src" | "data-src") {
                absolutize(&rewrite_base, value)
                    .filter(|url| crate::network::validate_url(url).is_ok())
                    .map(Cow::Owned)
            } else {
                Some(Cow::Borrowed(value))
            }
        });
    let clean = builder.clean(&filtered).to_string();
    let mut out = RE_IMG_TAG
        .replace_all(&clean, |caps: &regex::Captures| {
            let fragment = Html::parse_fragment(&caps[0]);
            let selector = Selector::parse("img").expect("img selector");
            let Some(img) = fragment.select(&selector).next() else {
                return String::new();
            };
            let Some(src) = img
                .value()
                .attr("src")
                .or_else(|| img.value().attr("data-src"))
            else {
                return String::new();
            };
            if crate::network::validate_url(src).is_err() {
                return String::new();
            }
            let alt = img.value().attr("alt").unwrap_or("");
            format!(
                "<img data-src=\"{}\" alt=\"{}\" loading=\"lazy\" referrerpolicy=\"no-referrer\">",
                html_escape_attr(src),
                html_escape_attr(alt)
            )
        })
        .into_owned();
    if out.len() > MAX_SANITIZED_HTML {
        let mut cut = MAX_SANITIZED_HTML;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        // Never leave a half-written tag at the truncation boundary.
        let prefix = &out[..cut];
        if prefix.rfind('<') > prefix.rfind('>') {
            cut = prefix.rfind('<').unwrap_or(cut);
        }
        out.truncate(cut);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_blocks_active_content_and_keeps_images_inert() {
        let html = r#"<p style=background:url(https://evil.com)>Read</p><svg><a href='javascript:alert(1)'>x</a></svg><iframe src='https://evil.com'></iframe><img src=/pic.png?a=1&amp;b=2 onerror=alert(1)><a href='javascript&#58;alert(1)' ping='https://evil.com'>x</a>"#;
        let clean = sanitize_html(html, &base());
        for forbidden in [
            "<svg",
            "<iframe",
            "style=",
            "onerror",
            "javascript",
            "ping=",
            "<img src=",
        ] {
            assert!(!clean.contains(forbidden), "{clean}");
        }
        assert!(clean.contains("data-src=\"https://example.com/pic.png?a=1&amp;b=2\""));
        assert_eq!(sanitize_html(&clean, &base()), clean);
    }

    fn base() -> Url {
        Url::parse("https://example.com/a/b/").unwrap()
    }

    #[test]
    fn extracts_and_absolutizes_links() {
        let html =
            r#"<a href="../guide?utm_source=x&id=7">Guide</a><a href="javascript:void(0)">x</a>"#;
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
        assert!(clean.contains("target=\"_blank\""));
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

    #[test]
    fn heavy_parsing_concurrency_is_bounded_and_permits_are_released() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|_| {
                let active = active.clone();
                let peak = peak.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    let _permit = ParsePermit::acquire();
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    active.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert!(peak.load(Ordering::SeqCst) <= PARSE_BUDGET.limit);
    }
}
