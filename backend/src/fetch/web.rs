//! Reading a web page: download it, then keep the article (or, for pages that aren't articles,
//! the title and description) as markdown. Extraction follows Atomic's
//! `crates/atomic-core/src/ingest/extract.rs`, using dom_smoothie (a Readability.js port).

use std::sync::LazyLock;
use std::time::Duration;

use dom_smoothie::{Config, Readability, TextMode};
use ureq::ResponseExt;

use crate::capture::detect::{TITLE_MAX, truncate};

const TIMEOUT: Duration = Duration::from_secs(10);
/// Bigger pages are almost never articles; don't download them.
const MAX_PAGE_BYTES: u64 = 5 * 1024 * 1024;
/// Shorter extracted text isn't worth more than the page's description.
const MIN_ARTICLE_CHARS: usize = 200;
/// A note this long is already a book chapter; the rest isn't needed to find related items.
const MAX_BODY_CHARS: usize = 200_000;
const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (compatible; Plexus/",
    env!("CARGO_PKG_VERSION"),
    "; +https://github.com/sahilf637/plexus)"
);

#[derive(Debug, PartialEq)]
pub struct Page {
    pub title: Option<String>,
    pub markdown: String,
}

#[derive(Debug, PartialEq)]
pub enum Failure {
    /// Might work later: offline, timed out, server error.
    Retry(String),
    /// Won't work by trying again: gone, not a web page, nothing readable on it.
    Unreadable(String),
}

pub fn fetch(url: &str) -> Result<Page, Failure> {
    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).user_agent(USER_AGENT).build().into()
    });
    let mut response = AGENT
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5")
        .call()
        .map_err(classify)?;

    let content_type = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !content_type.is_empty() && !content_type.contains("html") {
        // PDFs and images behind links are read once document support lands (M1).
        return Err(Failure::Unreadable(format!("not a web page ({content_type})")));
    }
    // Relative links in the article resolve against where redirects ended up.
    let final_url = response.get_uri().to_string();
    let bytes = response.body_mut().with_config().limit(MAX_PAGE_BYTES).read_to_vec().map_err(classify)?;
    extract(&String::from_utf8_lossy(&bytes), &final_url).ok_or_else(|| Failure::Unreadable("nothing readable".into()))
}

/// The page's article as markdown; for pages that aren't articles (home pages, apps), the
/// description. `None` when there's neither.
pub fn extract(html: &str, url: &str) -> Option<Page> {
    let config = Config { text_mode: TextMode::Markdown, ..Default::default() };
    let mut readability = Readability::new(html, Some(url), Some(config)).ok()?;
    let metadata = readability.get_article_metadata(readability.parse_json_ld());
    let article = readability.is_probably_readable().then(|| readability.parse().ok()).flatten();

    let title = article.as_ref().map(|a| a.title.as_str()).unwrap_or(&metadata.title);
    let title = clean(title);
    let text = article.as_ref().map(|a| a.text_content.trim()).filter(|t| t.chars().count() >= MIN_ARTICLE_CHARS);
    let markdown = match text {
        Some(text) => cap(&tidy(text)),
        None => clean(metadata.excerpt.as_deref().unwrap_or("")),
    };
    if title.is_empty() && markdown.is_empty() {
        return None;
    }
    let title = (!title.is_empty()).then(|| truncate(&title, TITLE_MAX));
    Some(Page { title, markdown })
}

fn classify(error: ureq::Error) -> Failure {
    match error {
        ureq::Error::StatusCode(code @ (408 | 429 | 500..)) => Failure::Retry(format!("HTTP {code}")),
        ureq::Error::StatusCode(code) => Failure::Unreadable(format!("HTTP {code}")),
        ureq::Error::BodyExceedsLimit(_) => Failure::Unreadable("page too large".into()),
        other => Failure::Retry(other.to_string()),
    }
}

/// Whitespace collapsed to single spaces.
fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Drops table rows with nothing in them (left over from layout tables and infoboxes) and runs
/// of blank lines.
fn tidy(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut blank = 0;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('|') && trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
            continue;
        }
        blank = if trimmed.is_empty() { blank + 1 } else { 0 };
        if blank < 2 {
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out.trim().to_string()
}

fn cap(text: &str) -> String {
    match text.char_indices().nth(MAX_BODY_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_an_article_as_markdown() {
        let paragraph = "Ownership is a set of rules that govern how a Rust program manages memory. \
                         Some languages have garbage collection; in others the programmer allocates \
                         and frees memory explicitly. Rust uses a third approach. ";
        let html = format!(
            "<html><head><title>What is Ownership? - The Rust Book</title></head><body>
             <nav><a href='/'>Home</a> <a href='/about'>About</a></nav>
             <article><h1>What is Ownership?</h1><p>{}</p><p>{}</p><p>See <a href='ch04-02.html'>references</a>.</p></article>
             <footer>© 2026</footer></body></html>",
            paragraph.repeat(3),
            paragraph.repeat(2)
        );
        let page = extract(&html, "https://doc.rust-lang.org/book/ch04-01.html").unwrap();
        assert!(page.title.as_deref().unwrap().contains("What is Ownership?"));
        assert!(page.markdown.contains("Rust uses a third approach"));
        assert!(page.markdown.contains("https://doc.rust-lang.org/book/ch04-02.html"), "{}", page.markdown);
        assert!(!page.markdown.contains("© 2026"));
    }

    #[test]
    fn empty_table_rows_and_blank_runs_are_dropped() {
        assert_eq!(tidy("Intro\n\n|   |   |\n| - | - |\n|  |  |\n\n\n\nText  \n| a | b |"), "Intro\n\nText\n| a | b |");
    }

    #[test]
    fn pages_that_arent_articles_keep_their_description() {
        let html = "<html><head><title>Plexus</title>
                    <meta name='description' content='Paste anything; it links itself.'></head>
                    <body><div id='app'></div></body></html>";
        let page = extract(html, "https://example.com/").unwrap();
        assert_eq!(page, Page { title: Some("Plexus".into()), markdown: "Paste anything; it links itself.".into() });
        assert_eq!(extract("<html><body></body></html>", "https://example.com/"), None);
    }

    #[test]
    fn failures_are_sorted_into_retry_or_give_up() {
        assert!(matches!(classify(ureq::Error::StatusCode(404)), Failure::Unreadable(_)));
        assert!(matches!(classify(ureq::Error::StatusCode(503)), Failure::Retry(_)));
        assert!(matches!(classify(ureq::Error::StatusCode(429)), Failure::Retry(_)));
        assert!(matches!(classify(ureq::Error::HostNotFound), Failure::Retry(_)));
    }

    #[test]
    #[ignore = "needs the network"]
    fn reads_a_real_page() {
        let page = fetch("https://en.wikipedia.org/wiki/Forgetting_curve").unwrap();
        println!("{:?}\n{}", page.title, &page.markdown[..page.markdown.len().min(600)]);
        assert!(page.markdown.contains("Ebbinghaus"));
    }
}
