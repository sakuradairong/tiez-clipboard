//! Clipboard privacy classification shared by TieZ desktop runtimes.

use regex::Regex;
use std::sync::OnceLock;

pub fn contains_sensitive_info(text: &str, kinds: &[String], custom_rules: &[String]) -> bool {
    static PHONE_RE: OnceLock<Regex> = OnceLock::new();
    static IDCARD_RE: OnceLock<Regex> = OnceLock::new();
    static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
    static SECRET_RE: OnceLock<Regex> = OnceLock::new();
    static URL_RE: OnceLock<Regex> = OnceLock::new();

    if text.len() > 5000 || text.starts_with("data:") {
        return false;
    }

    let has_kind = |kind: &str| kinds.iter().any(|value| value == kind);

    if has_kind("url") {
        let regex = URL_RE
            .get_or_init(|| Regex::new(r"(?i)(?:[a-zA-Z][a-zA-Z0-9+\-.]*://|www\.)\S+").unwrap());
        if regex.is_match(text) {
            return true;
        }
    }
    if has_kind("phone") {
        let regex = PHONE_RE.get_or_init(|| {
            Regex::new(r"(?:\+?86)?[-\s\(]*1[3-9]\d{1}[-\s\)]*\d{4}[-\s]*\d{4}").unwrap()
        });
        if regex.is_match(text) {
            return true;
        }
    }
    if has_kind("idcard") {
        let regex = IDCARD_RE.get_or_init(|| {
            Regex::new(
                r"\b[1-9]\d{5}[1-9]\d{3}((0\d)|(1[0-2]))(([0|1|2]\d)|3[0-1])\d{3}([0-9Xx])\b",
            )
            .unwrap()
        });
        if regex.is_match(text) {
            return true;
        }
    }
    if has_kind("email") {
        let regex = EMAIL_RE
            .get_or_init(|| Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap());
        if regex.is_match(text) {
            return true;
        }
    }
    if has_kind("secret") {
        let regex = SECRET_RE.get_or_init(|| Regex::new(r"(?ix)((?:sk|pk|ghp|gho|github_pat|AIza|AKIA|ya29)[-_][\w\-]{20,}|(?:password|secret|api[_-]?key|access[_-]?key|token|bearer)[\s:=]+[\w\-]{16,})").unwrap());
        if regex.is_match(text) {
            return true;
        }
    }
    if has_kind("password")
        && text.len() >= 8
        && text.len() <= 64
        && !text.contains(' ')
        && !text.contains('\n')
    {
        let has_upper = text.chars().any(char::is_uppercase);
        let has_lower = text.chars().any(char::is_lowercase);
        let has_digit = text.chars().any(char::is_numeric);
        let has_special = text.chars().any(|character| !character.is_alphanumeric());
        if has_upper && has_lower && has_digit && has_special {
            return true;
        }
    }

    custom_rules.iter().any(|rule| {
        Regex::new(rule)
            .map(|regex| regex.is_match(text))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn detects_each_production_privacy_kind() {
        assert!(contains_sensitive_info(
            "138 1234 5678",
            &kinds(&["phone"]),
            &[],
        ));
        assert!(contains_sensitive_info(
            "11010519491231002X",
            &kinds(&["idcard"]),
            &[],
        ));
        assert!(contains_sensitive_info(
            "person@example.com",
            &kinds(&["email"]),
            &[],
        ));
        assert!(contains_sensitive_info(
            "token=abcdefghijklmnop1234",
            &kinds(&["secret"]),
            &[],
        ));
        assert!(contains_sensitive_info(
            "StrongPass123!",
            &kinds(&["password"]),
            &[],
        ));
        assert!(contains_sensitive_info(
            "https://example.com",
            &kinds(&["url"]),
            &[],
        ));
    }

    #[test]
    fn custom_rules_match_and_invalid_rules_are_ignored() {
        assert!(contains_sensitive_info(
            "internal-ticket-42",
            &[],
            &["ticket-\\d+".to_owned()],
        ));
        assert!(!contains_sensitive_info(
            "ordinary text",
            &[],
            &["[invalid".to_owned()],
        ));
    }

    #[test]
    fn oversized_and_data_url_payloads_are_skipped() {
        assert!(!contains_sensitive_info(
            &"1".repeat(5001),
            &kinds(&["phone"]),
            &[],
        ));
        assert!(!contains_sensitive_info(
            "data:text/plain,person@example.com",
            &kinds(&["email"]),
            &[],
        ));
    }
}
