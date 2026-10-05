//! Working out what a capture is (text, code, a link, …) and a first title for it, without any AI.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Code,
    Url,
    Image,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    pub kind: Kind,
    /// For code, the language when it's recognisable.
    pub language: Option<String>,
    pub title: String,
}

const TITLE_MAX: usize = 80;

pub fn detect_text(text: &str) -> Detected {
    let trimmed = text.trim();
    if is_url(trimmed) {
        return Detected { kind: Kind::Url, language: None, title: url_title(trimmed) };
    }
    if let Some(language) = code_language(trimmed) {
        let language = (!language.is_empty()).then(|| language.to_string());
        return Detected { kind: Kind::Code, language, title: code_title(trimmed) };
    }
    Detected { kind: Kind::Text, language: None, title: text_title(trimmed) }
}

/// Image or other file, from its name and MIME type.
pub fn detect_file(name: &str, mime: &str) -> Detected {
    let kind = if mime.starts_with("image/") { Kind::Image } else { Kind::File };
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let title = truncate(stem.replace(['_', '-'], " ").trim(), TITLE_MAX);
    Detected { kind, language: None, title: if title.is_empty() { name.to_string() } else { title } }
}

fn is_url(text: &str) -> bool {
    static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://[^\s/$.?#][^\s]*$").unwrap());
    URL.is_match(text)
}

/// "https://www.example.com/blog/post-title?x=1" → "example.com/blog/post-title"
fn url_title(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest).trim_end_matches('/');
    truncate(rest.strip_prefix("www.").unwrap_or(rest), TITLE_MAX)
}

/// First meaningful line, without markdown markers.
fn text_title(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let line = line.trim_start_matches(['#', '>', '-', '*', ' ']).trim();
    if line.is_empty() { "Untitled".into() } else { truncate(line, TITLE_MAX) }
}

fn code_title(code: &str) -> String {
    let line = code
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("```") && !l.starts_with("#!"))
        .unwrap_or("Code snippet");
    truncate(line.trim_end_matches(['{', ':', ' ']), TITLE_MAX)
}

/// `Some(language)` if the text looks like code (`Some("")` when the language is unclear).
fn code_language(text: &str) -> Option<&'static str> {
    if text.starts_with("```") {
        let info = text.lines().next().unwrap_or("").trim_start_matches('`').trim();
        return Some(fence_language(&info.to_ascii_lowercase()));
    }
    if let Some(shebang) = text.lines().next().filter(|l| l.starts_with("#!")) {
        return Some(match () {
            _ if shebang.contains("python") => "python",
            _ if shebang.contains("node") => "javascript",
            _ => "bash",
        });
    }
    if (text.starts_with('{') || text.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(text).is_ok_and(|v| v.is_object() || v.is_array())
    {
        return Some("json");
    }

    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let codey = lines.iter().filter(|l| looks_like_code_line(l)).count();
    let is_code = match lines.len() {
        0 => false,
        1 => STRONG_SINGLE_LINE.is_match(lines[0]),
        n => codey * 2 >= n,
    };
    is_code.then(|| guess_language(text))
}

/// The language named after a code fence (```ts), including common short names.
fn fence_language(info: &str) -> &'static str {
    let name = match info {
        "rs" => "rust",
        "py" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "golang" => "go",
        "sh" | "shell" | "zsh" | "console" => "bash",
        other => other,
    };
    LANGUAGES.iter().find(|(lang, _)| *lang == name).map_or("", |(lang, _)| lang)
}

static STRONG_SINGLE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(fn \w+\s*\(|def \w+\s*\(|func \w+\s*\(|(const|let|var) \w+\s*=|import .+ from |from \S+ import |#include\s*<|(?i:select) .+ (?i:from) |\$ \S+)",
    )
    .unwrap()
});

fn looks_like_code_line(line: &str) -> bool {
    static CODE_LINE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"([;{}]\s*$|^\s*[}\])]|=>|::|->|==|!=|&&|\|\||^\s*(fn|def|func|class|import|export|return|if|for|while|let|const|var|pub|use|package|#include|public|private)\b|^\s*(//|/\*)|^\s*<\/?\w+[^>]*>\s*$)",
        )
        .unwrap()
    });
    CODE_LINE.is_match(line)
}

/// (language, patterns that point to it). The first language with the most matches wins.
const LANGUAGES: &[(&str, &[&str])] = &[
    ("rust", &["fn ", "let mut ", "impl ", "pub fn", "::", "-> ", "&self", "use std"]),
    ("python", &["def ", "import ", "self.", "elif ", "print(", "None", "__init__"]),
    ("typescript", &["interface ", ": string", ": number", "export type", "readonly "]),
    ("javascript", &["const ", "=> ", "function ", "console.log", "require(", "export default"]),
    ("go", &["func ", "package ", ":= ", "fmt.", "err != nil"]),
    ("java", &["public class", "System.out", "private ", "void ", "@Override"]),
    ("c", &["#include", "printf(", "int main", "->", "malloc("]),
    ("html", &["<div", "</", "<html", "<span", "class=\""]),
    ("css", &["{", "px;", "color:", "margin:", "display:"]),
    ("sql", &["SELECT ", "FROM ", "WHERE ", "INSERT ", "JOIN "]),
    ("bash", &["$ ", "sudo ", "echo ", "export ", " | ", "apt ", "npm ", "cargo "]),
    ("json", &[]),
];

fn guess_language(code: &str) -> &'static str {
    let mut best = ("", 0);
    for (name, patterns) in LANGUAGES {
        let hits = patterns.iter().filter(|p| code.contains(*p)).count();
        if hits > best.1 {
            best = (name, hits);
        }
    }
    // One stray match isn't enough to name a language.
    if best.1 >= 2 { best.0 } else { "" }
}

/// Cuts at a word boundary and adds an ellipsis.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head).trim_end();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(text: &str) -> Kind {
        detect_text(text).kind
    }

    #[test]
    fn links() {
        let d = detect_text("  https://www.example.com/blog/rust-ownership?ref=x  ");
        assert_eq!(d.kind, Kind::Url);
        assert_eq!(d.title, "example.com/blog/rust-ownership");
        assert_eq!(kind("see https://example.com for details"), Kind::Text);
    }

    #[test]
    fn code() {
        let rust = "fn main() {\n    let mut x = 5;\n    println!(\"{x}\");\n}";
        let d = detect_text(rust);
        assert_eq!(d.kind, Kind::Code);
        assert_eq!(d.language.as_deref(), Some("rust"));
        assert_eq!(d.title, "fn main()");

        let python = "def greet(name):\n    print(f'hi {name}')\n    return None";
        assert_eq!(detect_text(python).language.as_deref(), Some("python"));
        assert_eq!(detect_text("```ts\nconst x = 1\n```").language.as_deref(), Some("typescript"));
        assert_eq!(detect_text(r#"{"a": [1, 2]}"#).language.as_deref(), Some("json"));
        assert_eq!(detect_text("#!/bin/bash\necho hi").language.as_deref(), Some("bash"));
        assert_eq!(kind("const total = items.length"), Kind::Code);
    }

    #[test]
    fn prose_is_text() {
        let prose = "Ownership is Rust's most unique feature.\nIt enables memory safety without a garbage collector.\nLet's look at how it works.";
        assert_eq!(kind(prose), Kind::Text);
        assert_eq!(detect_text(prose).title, "Ownership is Rust's most unique feature.");
        assert_eq!(detect_text("# Meeting notes\n- ship it").title, "Meeting notes");
        assert_eq!(kind("Remember to buy milk; and eggs"), Kind::Text);
    }

    #[test]
    fn files() {
        assert_eq!(detect_file("whiteboard_sketch.png", "image/png").kind, Kind::Image);
        let pdf = detect_file("Q3-report-final.pdf", "application/pdf");
        assert_eq!((pdf.kind, pdf.title.as_str()), (Kind::File, "Q3 report final"));
    }

    #[test]
    fn long_titles_are_cut_at_a_word() {
        let title = text_title(&"word ".repeat(40));
        assert!(title.ends_with('…') && title.chars().count() <= TITLE_MAX + 1);
    }
}
