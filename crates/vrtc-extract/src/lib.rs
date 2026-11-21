//! Native HTML parsing and content extraction for VRTc.
//!
//! This engine adopts a browser-grade HTML5 parser and implements VRTc-owned
//! candidate scoring, metadata extraction, link normalization, and text blocks.

use std::collections::{BTreeMap, HashSet};
use std::time::SystemTime;

use scraper::{ElementRef, Html, Selector};
use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct ExtractionConfig {
    pub max_html_bytes: usize,
    pub min_text_chars: usize,
    pub max_links: usize,
    pub max_metadata_entries: usize,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            max_html_bytes: 8 * 1024 * 1024,
            min_text_chars: 80,
            max_links: 500,
            max_metadata_entries: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedHeading {
    pub level: u8,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedLink {
    pub text: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedDocument {
    pub source_url: Option<String>,
    pub canonical_url: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub language: Option<String>,
    pub text: String,
    pub markdown: String,
    pub headings: Vec<ExtractedHeading>,
    pub links: Vec<ExtractedLink>,
    pub metadata: BTreeMap<String, String>,
    pub extraction_method: String,
    pub confidence: f32,
    pub extracted_at: SystemTime,
}

pub struct ExtractionEngine {
    config: ExtractionConfig,
}

impl ExtractionEngine {
    pub fn new(config: ExtractionConfig) -> Result<Self, VrtCError> {
        if config.max_html_bytes == 0
            || config.min_text_chars == 0
            || config.max_links == 0
            || config.max_metadata_entries == 0
        {
            return Err(extract_error(
                "INVALID_EXTRACTION_CONFIG",
                "HTML, text, link, and metadata limits must be positive",
                ErrorCategory::Validation,
            ));
        }
        Ok(Self { config })
    }

    /// Extract readable content and provenance from one HTML document.
    pub fn extract(
        &self,
        html: &str,
        source_url: Option<&str>,
    ) -> Result<ExtractedDocument, VrtCError> {
        if html.len() > self.config.max_html_bytes {
            return Err(extract_error(
                "HTML_INPUT_TOO_LARGE",
                "The HTML document exceeds the configured input limit",
                ErrorCategory::ResourceExhausted,
            ));
        }

        let document = Html::parse_document(html);
        let base = source_url.and_then(|value| Url::parse(value).ok());
        let metadata = extract_metadata(&document, self.config.max_metadata_entries);
        let candidate = choose_candidate(&document).ok_or_else(|| {
            extract_error(
                "CONTENT_CANDIDATE_NOT_FOUND",
                "No suitable content container was found in the HTML document",
                ErrorCategory::NotFound,
            )
        })?;

        let (text, markdown) = extract_blocks(candidate);
        let text = if text.is_empty() {
            normalize_text(candidate.text().collect::<String>())
        } else {
            text
        };
        if text.chars().count() < self.config.min_text_chars {
            return Err(extract_error(
                "CONTENT_TOO_SHORT",
                "The extracted content is below the configured minimum text length",
                ErrorCategory::NotFound,
            ));
        }

        let title = metadata_value(&metadata, &["og:title", "twitter:title", "title"])
            .or_else(|| select_text(&document, "title"))
            .or_else(|| select_text(&document, "h1"));
        let description = metadata_value(&metadata, &["description", "og:description", "twitter:description"]);
        let author = metadata_value(&metadata, &["author", "article:author", "parsely-author"]);
        let published_at = metadata_value(
            &metadata,
            &["article:published_time", "datepublished", "date", "pubdate"],
        );
        let language = select_attr(&document, "html", "lang");
        let canonical_url = select_attr(&document, "link[rel='canonical']", "href")
            .and_then(|value| normalize_url(&value, base.as_ref()));
        let headings = extract_headings(candidate);
        let links = extract_links(candidate, base.as_ref(), self.config.max_links);
        let confidence = estimate_confidence(&text, headings.len());

        Ok(ExtractedDocument {
            source_url: source_url.map(str::to_owned),
            canonical_url,
            title,
            description,
            author,
            published_at,
            language,
            text,
            markdown,
            headings,
            links,
            metadata,
            extraction_method: "vrtc-native-heuristic-v1".to_owned(),
            confidence,
            extracted_at: SystemTime::now(),
        })
    }
}

fn choose_candidate<'a>(document: &'a Html) -> Option<ElementRef<'a>> {
    let selector = Selector::parse(
        "article, main, [role='main'], [itemprop='articleBody'], #content, .content, .article, .post, .entry-content, body",
    )
    .ok()?;
    document.select(&selector).max_by_key(|element| candidate_score(*element))
}

fn candidate_score(element: ElementRef<'_>) -> i64 {
    let text = normalize_text(element.text().collect::<String>());
    let text_length = text.chars().count() as i64;
    let paragraphs = Selector::parse("p")
        .ok()
        .map(|selector| element.select(&selector).count() as i64)
        .unwrap_or_default();
    let headings = Selector::parse("h1,h2,h3")
        .ok()
        .map(|selector| element.select(&selector).count() as i64)
        .unwrap_or_default();
    let link_chars = Selector::parse("a")
        .ok()
        .map(|selector| {
            element
                .select(&selector)
                .map(|link| link.text().collect::<String>().chars().count() as i64)
                .sum::<i64>()
        })
        .unwrap_or_default();
    let class_and_id = format!(
        "{} {}",
        element.value().attr("class").unwrap_or_default(),
        element.value().attr("id").unwrap_or_default()
    )
    .to_ascii_lowercase();
    let positive = ["article", "content", "post", "entry", "story", "main"]
        .iter()
        .filter(|hint| class_and_id.contains(**hint))
        .count() as i64;
    let negative = ["nav", "footer", "sidebar", "comment", "related", "menu", "advert", "header"]
        .iter()
        .filter(|hint| class_and_id.contains(**hint))
        .count() as i64;
    let excluded_text = Selector::parse("script,style,noscript,template,svg")
        .ok()
        .map(|selector| {
            element
                .select(&selector)
                .map(|node| node.text().collect::<String>().chars().count() as i64)
                .sum::<i64>()
        })
        .unwrap_or_default();

    text_length + paragraphs * 180 + headings * 55 + positive * 260 - link_chars * 2
        - negative * 650 - excluded_text * 2
}

fn extract_blocks(candidate: ElementRef<'_>) -> (String, String) {
    let Ok(selector) = Selector::parse("h1,h2,h3,h4,h5,h6,p,li,blockquote,pre") else {
        return (String::new(), String::new());
    };
    let mut seen = HashSet::new();
    let mut plain_blocks = Vec::new();
    let mut markdown_blocks = Vec::new();

    for element in candidate.select(&selector) {
        let text = normalize_text(element.text().collect::<String>());
        if text.is_empty() || !seen.insert(text.clone()) {
            continue;
        }
        let name = element.value().name();
        let markdown = match name {
            "h1" => format!("# {text}"),
            "h2" => format!("## {text}"),
            "h3" => format!("### {text}"),
            "h4" => format!("#### {text}"),
            "h5" => format!("##### {text}"),
            "h6" => format!("###### {text}"),
            "li" => format!("- {text}"),
            "blockquote" => format!("> {text}"),
            "pre" => format!("\n\`\`\`\n{text}\n\`\`\`"),
            _ => text.clone(),
        };
        plain_blocks.push(text);
        markdown_blocks.push(markdown);
    }
    (plain_blocks.join("\n\n"), markdown_blocks.join("\n\n"))
}

fn extract_headings(candidate: ElementRef<'_>) -> Vec<ExtractedHeading> {
    let Ok(selector) = Selector::parse("h1,h2,h3,h4,h5,h6") else {
        return Vec::new();
    };
    candidate
        .select(&selector)
        .filter_map(|element| {
            let level = element.value().name().trim_start_matches('h').parse::<u8>().ok()?;
            let text = normalize_text(element.text().collect::<String>());
            (!text.is_empty()).then_some(ExtractedHeading { level, text })
        })
        .collect()
}

fn extract_links(
    candidate: ElementRef<'_>,
    base: Option<&Url>,
    max_links: usize,
) -> Vec<ExtractedLink> {
    let Ok(selector) = Selector::parse("a[href]") else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    candidate
        .select(&selector)
        .filter_map(|element| {
            if seen.len() >= max_links {
                return None;
            }
            let raw_url = element.value().attr("href")?;
            let url = normalize_url(raw_url, base)?;
            if !seen.insert(url.clone()) {
                return None;
            }
            Some(ExtractedLink {
                text: normalize_text(element.text().collect::<String>()),
                url,
            })
        })
        .collect()
}

fn extract_metadata(document: &Html, max_entries: usize) -> BTreeMap<String, String> {
    let Ok(selector) = Selector::parse("meta[name],meta[property],meta[itemprop]") else {
        return BTreeMap::new();
    };
    document
        .select(&selector)
        .filter_map(|element| {
            let key = element
                .value()
                .attr("property")
                .or_else(|| element.value().attr("name"))
                .or_else(|| element.value().attr("itemprop"))?
                .trim()
                .to_ascii_lowercase();
            let value = element.value().attr("content")?.trim();
            if key.is_empty() || value.is_empty() {
                return None;
            }
            Some((key, value.to_owned()))
        })
        .take(max_entries)
        .collect()
}

fn metadata_value(metadata: &BTreeMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| metadata.get(*key).cloned())
        .filter(|value| !value.trim().is_empty())
}

fn select_text(document: &Html, query: &str) -> Option<String> {
    let selector = Selector::parse(query).ok()?;
    document
        .select(&selector)
        .next()
        .map(|element| normalize_text(element.text().collect::<String>()))
        .filter(|value| !value.is_empty())
}

fn select_attr(document: &Html, query: &str, attribute: &str) -> Option<String> {
    let selector = Selector::parse(query).ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr(attribute))
        .map(str::to_owned)
        .filter(|value| !value.trim().is_empty())
}

fn normalize_url(raw: &str, base: Option<&Url>) -> Option<String> {
    let url = match Url::parse(raw) {
        Ok(url) => url,
        Err(_) => base?.join(raw).ok()?,
    };
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn normalize_text(value: String) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn estimate_confidence(text: &str, heading_count: usize) -> f32 {
    let length = text.chars().count();
    match (length, heading_count) {
        (2000.., 2..) => 0.92,
        (1000.., _) => 0.85,
        (500.., _) => 0.78,
        (200.., _) => 0.68,
        _ => 0.55,
    }
}

fn extract_error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(code, message, category, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_article_metadata_and_main_content() {
        let html = r#"
            <html lang="en">
              <head>
                <title>Fallback title</title>
                <meta property="og:title" content="Native extraction test">
                <meta name="description" content="A test page">
                <meta name="author" content="VRTc">
              </head>
              <body>
                <nav><p>Home Search Contact navigation noise</p></nav>
                <main class="article-content">
                  <h1>Native extraction test</h1>
                  <p>This is the first paragraph containing enough readable information for the extraction engine.</p>
                  <p>This is the second paragraph with additional context, details, and useful source material.</p>
                  <a href="/related">Related article</a>
                </main>
                <footer><p>Copyright and footer navigation</p></footer>
              </body>
            </html>
        "#;
        let engine = ExtractionEngine::new(ExtractionConfig::default()).unwrap();
        let result = engine
            .extract(html, Some("https://example.org/news/story"))
            .unwrap();
        assert_eq!(result.title.as_deref(), Some("Native extraction test"));
        assert_eq!(result.author.as_deref(), Some("VRTc"));
        assert_eq!(result.language.as_deref(), Some("en"));
        assert!(result.text.contains("first paragraph"));
        assert!(!result.text.contains("footer navigation"));
        assert_eq!(result.links[0].url, "https://example.org/related");
        assert!(result.markdown.contains("# Native extraction test"));
    }

    #[test]
    fn rejects_oversized_html_input() {
        let engine = ExtractionEngine::new(ExtractionConfig {
            max_html_bytes: 8,
            ..ExtractionConfig::default()
        })
        .unwrap();
        assert!(engine.extract("<html>long input</html>", None).is_err());
    }

    #[test]
    fn rejects_short_or_empty_content() {
        let engine = ExtractionEngine::new(ExtractionConfig::default()).unwrap();
        assert!(engine.extract("<html><body><main>short</main></body></html>", None).is_err());
    }
}
