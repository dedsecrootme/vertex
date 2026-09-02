use regex::Regex;
use scraper::{Html, Selector};
use std::sync::LazyLock;

/// Motifs des blocs de page Ã  retirer du contenu (mÃ©tadonnÃ©es, navigation,
/// scripts/analytics, formulaires de cookies, iframesâ€¦).
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

/// Retire du HTML les Ã©lÃ©ments hors contenu (navigation, scripts, pub, etc.).
fn remove_junk(html: &str) -> String {
    let mut out = html.to_string();
    for re in JUNK_PATTERNS.iter() {
        out = re.replace_all(&out, " ").to_string();
    }
    out
}

/// RÃ©duit tout espace (retours Ã  la ligne, tabulations, espaces rÃ©pÃ©tÃ©s) en
/// un seul espace et supprime les espaces aux extrÃ©mitÃ©s.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::new();
    let mut prev_space = true;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Extrait le titre d'une page HTML.
pub fn extract_title(html: &str) -> String {
    let document = Html::parse_document(html);
    let selector = Selector::parse("title").expect("sÃ©lecteur title invalide");

    match document.select(&selector).next() {
        Some(element) => element.text().collect::<String>().trim().to_string(),
        None => String::from("Sans titre"),
    }
}

/// Extrait les liens prÃ©sents dans la page.
pub fn extract_links(html: &str) -> Vec<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("a").expect("sÃ©lecteur lien invalide");
    let mut links = Vec::new();

    for element in document.select(&selector) {
        if let Some(link) = element.value().attr("href") {
            links.push(link.to_string());
        }
    }
    links
}

/// Extrait le texte visible et propre de la page (sans navigation ni scripts).
pub fn extract_text(html: &str) -> String {
    let sanitized = remove_junk(html);
    let document = Html::parse_document(&sanitized);
    let selector = Selector::parse("body").expect("sÃ©lecteur body invalide");
    let mut text = String::new();

    for element in document.select(&selector) {
        text.push_str(&element.text().collect::<Vec<_>>().join(" "));
    }

    collapse_whitespace(&text)
}