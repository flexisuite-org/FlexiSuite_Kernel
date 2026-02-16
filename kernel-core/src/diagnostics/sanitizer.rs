use regex::{Regex, Captures};
use once_cell::sync::Lazy;
use serde_json::Value;
use url::Url;

static EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}").unwrap());
// Bearer token: Bearer <token>
static BEARER_TOKEN_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"Bearer\s+[a-zA-Z0-9\-\._~\+\/]+=*").unwrap());
// Basic phone detection (generic)
#[allow(dead_code)]
static PHONE_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\+?[\d\s\-\(\)]{7,15}").unwrap());
// API Key / Secret patterns
static API_KEY_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)(api_key|apikey|secret|token)['"]?\s*[:=]\s*['"]?([a-zA-Z0-9\-_]{16,})['"]?"#).unwrap());
// URL Regex (simplified for text scanning)
static URL_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(https?://[^\s<>"']+)"#).unwrap());

pub struct PIISanitizer;

impl PIISanitizer {
    /// Masks PII in the given string.
    pub fn sanitize_text(text: &str) -> String {
        let mut sanitized = text.to_string();

        // Mask Emails
        sanitized = EMAIL_REGEX.replace_all(&sanitized, "***EMAIL***").to_string();

        // Mask Bearer Tokens
        sanitized = BEARER_TOKEN_REGEX.replace_all(&sanitized, "Bearer ***TOKEN***").to_string();

        // Mask Phone Numbers
        // sanitized = PHONE_REGEX.replace_all(&sanitized, "***PHONE***").to_string();

        // Mask API Keys (Simple heuristic)
        sanitized = API_KEY_REGEX.replace_all(&sanitized, "$1: ***SECRET***").to_string();

        // Sanitize URLs in text
        sanitized = URL_REGEX.replace_all(&sanitized, |caps: &Captures| {
            let url_str = &caps[0];
            if let Ok(mut url) = Url::parse(url_str) {
                url.set_query(None);
                url.set_fragment(None);
                url.to_string()
            } else {
                url_str.to_string()
            }
        }).to_string();

        sanitized
    }

    /// Sanitizes a URL string by removing query parameters and fragments.
    pub fn sanitize_url(url_str: &str) -> String {
        if let Ok(mut url) = Url::parse(url_str) {
            // Remove query and fragment
            url.set_query(None);
            url.set_fragment(None);

            // Run text sanitization on the result to catch PII in path
            Self::sanitize_text(url.as_str())
        } else {
            Self::sanitize_text(url_str)
        }
    }

    /// Recursively sanitizes a JSON Value.
    pub fn sanitize_value(v: &mut Value) {
        match v {
            Value::String(s) => {
                if s.starts_with("http://") || s.starts_with("https://") {
                    *s = Self::sanitize_url(s);
                } else {
                    *s = Self::sanitize_text(s);
                }
            }
            Value::Array(arr) => {
                for i in arr {
                    Self::sanitize_value(i);
                }
            }
            Value::Object(obj) => {
                for (_, val) in obj {
                    Self::sanitize_value(val);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_sanitize_text_email() {
        let input = "Contact me at test@example.com for more info.";
        let expected = "Contact me at ***EMAIL*** for more info.";
        assert_eq!(PIISanitizer::sanitize_text(input), expected);
    }

    #[test]
    fn test_sanitize_text_bearer_token() {
        let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let expected = "Authorization: Bearer ***TOKEN***";
        assert_eq!(PIISanitizer::sanitize_text(input), expected);
    }

    #[test]
    fn test_sanitize_text_api_key() {
        let input = "api_key = 'abcdef1234567890abcdef1234567890'";
        let expected = "api_key: ***SECRET***";
        assert_eq!(PIISanitizer::sanitize_text(input), expected);
    }

    #[test]
    fn test_sanitize_text_url_in_dom() {
        let input = "<div><a href=\"https://example.com/sensitive?token=123#fragment\">Link</a></div>";
        // The regex (https?://[^\s<>]+) stops at " (quote) or < (tag) or space.
        // In "href=\"https://...\"", the quote should stop it.
        // Let's verify the behavior for typical HTML.
        let expected = "<div><a href=\"https://example.com/sensitive\">Link</a></div>";
        assert_eq!(PIISanitizer::sanitize_text(input), expected);
    }

    #[test]
    fn test_sanitize_url() {
        let input = "https://example.com/path?query=secret#fragment";
        let expected = "https://example.com/path";
        assert_eq!(PIISanitizer::sanitize_url(input), expected);

        // Nested PII in path
        let input_with_email = "https://example.com/user/test@example.com?query=123";
        // sanitize_url calls sanitize_text on the cleaned URL, so email should be masked.
        let expected_with_email = "https://example.com/user/***EMAIL***";
        assert_eq!(PIISanitizer::sanitize_url(input_with_email), expected_with_email);
    }

    #[test]
    fn test_sanitize_value_recursive() {
        let mut input = json!({
            "user": {
                "email": "user@example.com",
                "token": "Bearer token123",
                "profile_url": "https://example.com/profile?id=123"
            },
            "logs": [
                "Error sending to admin@example.com",
                "Retry"
            ]
        });

        PIISanitizer::sanitize_value(&mut input);

        let expected = json!({
            "user": {
                "email": "***EMAIL***",
                "token": "Bearer ***TOKEN***",
                "profile_url": "https://example.com/profile"
            },
            "logs": [
                "Error sending to ***EMAIL***",
                "Retry"
            ]
        });

        assert_eq!(input, expected);
    }
}
