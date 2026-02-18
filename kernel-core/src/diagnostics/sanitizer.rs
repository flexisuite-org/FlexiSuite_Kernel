use regex::{Captures, Regex};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;
use url::Url;

static EMAIL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}").unwrap());
// Bearer token: Bearer <token>
static BEARER_TOKEN_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Bearer\s+[a-zA-Z0-9\-\._~\+\/]+=*").unwrap());
// Basic phone detection (generic)
#[allow(dead_code)]
static PHONE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\+?[\d\s\-\(\)]{7,15}").unwrap());
// API Key / Secret patterns
static API_KEY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(api_key|apikey|secret|token)['"]?\s*[:=]\s*['"]?([a-zA-Z0-9\-_]{16,})['"]?"#)
        .unwrap()
});
// URL Regex (simplified for text scanning)
static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(https?://[^\s<>"']+)"#).unwrap());

/// Maximum recursion depth for JSON value sanitization.
/// Prevents stack overflow on deeply nested payloads.
const MAX_DEPTH: usize = 32;

pub struct PIISanitizer;

impl PIISanitizer {
    /// Masks PII in the given string.
    pub fn sanitize_text(text: &str) -> String {
        let mut sanitized = text.to_string();

        // Mask Emails
        sanitized = EMAIL_REGEX
            .replace_all(&sanitized, "***EMAIL***")
            .to_string();

        // Mask Bearer Tokens
        sanitized = BEARER_TOKEN_REGEX
            .replace_all(&sanitized, "Bearer ***TOKEN***")
            .to_string();

        // Mask Phone Numbers
        // sanitized = PHONE_REGEX.replace_all(&sanitized, "***PHONE***").to_string();

        // Mask API Keys (Simple heuristic)
        sanitized = API_KEY_REGEX
            .replace_all(&sanitized, "$1: ***SECRET***")
            .to_string();

        // Sanitize URLs in text
        sanitized = URL_REGEX
            .replace_all(&sanitized, |caps: &Captures| {
                let url_str = &caps[0];
                if let Ok(mut url) = Url::parse(url_str) {
                    url.set_query(None);
                    url.set_fragment(None);
                    url.to_string()
                } else {
                    url_str.to_string()
                }
            })
            .to_string();

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

    /// Recursively sanitizes a JSON Value with depth limiting.
    ///
    /// Public entry point that enforces `MAX_DEPTH` to prevent stack overflow
    /// on adversarially nested payloads.
    pub fn sanitize_value(v: &mut Value) {
        Self::sanitize_value_inner(v, MAX_DEPTH);
    }

    fn sanitize_value_inner(v: &mut Value, remaining_depth: usize) {
        if remaining_depth == 0 {
            *v = Value::String("<max_depth_reached>".to_string());
            return;
        }

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
                    Self::sanitize_value_inner(i, remaining_depth - 1);
                }
            }
            Value::Object(obj) => {
                // Sanitize both keys and values, rebuilding the map
                let entries: Vec<(String, Value)> = obj
                    .into_iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                obj.clear();
                let mut key_counters: HashMap<String, usize> = HashMap::new();

                for (key, mut val) in entries {
                    let sanitized_key = Self::sanitize_text(&key);
                    Self::sanitize_value_inner(&mut val, remaining_depth - 1);
                    let key_index = key_counters.entry(sanitized_key.clone()).or_insert(0);
                    let output_key = if *key_index == 0 {
                        sanitized_key
                    } else {
                        format!("{}_{}", sanitized_key, *key_index)
                    };
                    *key_index += 1;
                    obj.insert(output_key, val);
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
        let input =
            "<div><a href=\"https://example.com/sensitive?token=123#fragment\">Link</a></div>";
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
        assert_eq!(
            PIISanitizer::sanitize_url(input_with_email),
            expected_with_email
        );
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

    #[test]
    fn test_sanitize_value_depth_limit() {
        // Build a deeply nested JSON structure that exceeds MAX_DEPTH
        let mut value = json!("leaf");
        for _ in 0..(MAX_DEPTH + 5) {
            value = json!({ "nested": value });
        }

        // Should not stack overflow
        PIISanitizer::sanitize_value(&mut value);

        // Verify that the deeply nested part was replaced
        let serialized = serde_json::to_string(&value).unwrap();
        assert!(
            serialized.contains("<max_depth_reached>"),
            "Expected depth-limited placeholder in output"
        );
    }

    #[test]
    fn test_sanitize_value_object_key_sanitization() {
        let mut input = json!({
            "user@example.com": "value1",
            "normal_key": "value2"
        });

        PIISanitizer::sanitize_value(&mut input);

        // The key with email should be sanitized
        assert!(
            input.get("***EMAIL***").is_some(),
            "Key with email should be sanitized"
        );
        assert!(
            input.get("normal_key").is_some(),
            "Normal key should remain"
        );
        // Original key should be gone
        assert!(
            input.get("user@example.com").is_none(),
            "Original email key should be removed"
        );
    }

    #[test]
    fn test_sanitize_value_object_key_collision() {
        // Two different keys that sanitize to the same result
        let mut input = json!({
            "a@b.com": "val1",
            "c@d.com": "val2"
        });

        PIISanitizer::sanitize_value(&mut input);

        // Both values should be preserved (one with suffix)
        let obj = input.as_object().unwrap();
        assert_eq!(
            obj.len(),
            2,
            "Both entries should be preserved despite key collision"
        );
        assert!(obj.contains_key("***EMAIL***"), "First sanitized key");
        assert!(obj.contains_key("***EMAIL***_1"), "Collision-suffixed key");
    }
}
