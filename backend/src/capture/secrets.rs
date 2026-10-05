//! Spotting passwords, API keys and tokens before anything is saved. Clipboards are full of them,
//! and a saved secret would end up in a plain file and possibly in an AI request.

use std::sync::LazyLock;

use regex::Regex;

/// Returns what kind of secret the text seems to contain, phrased for the UI
/// ("This looks like {reason}"), or `None`.
pub fn find_secret(text: &str) -> Option<&'static str> {
    static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        [
            (r"-----BEGIN [A-Z ]*PRIVATE KEY-----", "a private key"),
            (r"\bAKIA[0-9A-Z]{16}\b", "an AWS access key"),
            (r"\b(gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{50,})", "a GitHub token"),
            (r"\bxox[abprs]-[A-Za-z0-9-]{10,}", "a Slack token"),
            (r"\bAIza[0-9A-Za-z_\-]{35}\b", "a Google API key"),
            (r"\b(sk|rk)_live_[0-9A-Za-z]{20,}", "a Stripe key"),
            (r"\bsk-[A-Za-z0-9_\-]{20,}", "an API key"),
            (r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}", "a login token (JWT)"),
            (
                r#"(?i)\b(api[_-]?key|secret|token|password|passwd|pwd)\s*[:=]\s*["']?[^\s"']{8,}"#,
                "a password or key",
            ),
        ]
        .into_iter()
        .map(|(pattern, reason)| (Regex::new(pattern).unwrap(), reason))
        .collect()
    });

    PATTERNS
        .iter()
        .find(|(pattern, _)| pattern.is_match(text))
        .map(|(_, reason)| *reason)
        .or_else(|| is_random_token(text.trim()).then_some("a password or key"))
}

/// A single long word mixing letters, digits and symbols with high entropy: what generated
/// passwords and keys look like. Only applied to single-word captures, to keep prose and code
/// (full of hashes and ids) from tripping it.
fn is_random_token(text: &str) -> bool {
    if text.len() < 20 || text.len() > 200 || text.contains(char::is_whitespace) || text.contains("://") {
        return false;
    }
    let classes = [
        text.chars().any(|c| c.is_ascii_lowercase()),
        text.chars().any(|c| c.is_ascii_uppercase()),
        text.chars().any(|c| c.is_ascii_digit()),
        text.chars().any(|c| !c.is_ascii_alphanumeric()),
    ];
    classes.iter().filter(|&&has| has).count() >= 3 && entropy(text) >= 3.5
}

/// Shannon entropy in bits per character.
fn entropy(text: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    for c in text.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    let len = text.chars().count() as f64;
    counts.values().map(|&n| n as f64 / len).map(|p| -p * p.log2()).sum()
}

#[cfg(test)]
mod tests {
    use super::find_secret;

    #[test]
    fn finds_known_secrets() {
        assert_eq!(find_secret("AKIAIOSFODNN7EXAMPLE"), Some("an AWS access key"));
        assert_eq!(find_secret("token: ghp_abcdefghijklmnopqrstuvwxyz0123456789"), Some("a GitHub token"));
        assert_eq!(find_secret("OPENAI_KEY=sk-proj-abcdefghijklmnopqrstuvwx"), Some("an API key"));
        assert_eq!(find_secret("-----BEGIN OPENSSH PRIVATE KEY-----\nabc"), Some("a private key"));
        assert_eq!(find_secret(r#"password = "hunter2hunter2""#), Some("a password or key"));
        assert_eq!(find_secret("Xk9#mP2$vL7qR4@nT8wZ"), Some("a password or key"));
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        assert_eq!(find_secret("The token economy is a theory in behavioural psychology."), None);
        assert_eq!(find_secret("https://example.com/a1B2c3D4e5F6g7H8i9J0?x=1"), None);
        assert_eq!(find_secret("commit 9fceb02d0ae598e95dc970b74767f19372d61af8"), None);
        assert_eq!(find_secret("let password_hint = user.hint();"), None);
        assert_eq!(find_secret("supercalifragilisticexpialidocious"), None);
    }
}
