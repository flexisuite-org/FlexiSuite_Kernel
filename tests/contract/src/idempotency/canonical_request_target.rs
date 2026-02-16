#[cfg(test)]
mod tests {
    use kernel_core::idempotency::{canonicalize_request_target, check_idempotency_conflict};

    #[test]
    fn test_idempotency_canonical_request_target() {
        let cases = vec![
            // Basic
            ("/v1/items", None, "/v1/items"),
            ("/v1/items/", None, "/v1/items"), // Trailing slash removal
            ("/", None, "/"),                  // Root preserved
            // Query Sorting
            ("/search", Some("b=2&a=1"), "/search?a=1&b=2"),
            ("/search", Some("id=3&id=1&id=2"), "/search?id=1&id=2&id=3"), // Value sorting
            // Empty Query handling
            ("/path", Some(""), "/path"),
            ("/path", Some("a=1&"), "/path?&a=1"), // Trailing & -> empty key -> sorted to first
            // Encoding/Case Preservation (MUST NOT decode/lowercase)
            ("/v1/User/123", None, "/v1/User/123"),
            (
                "/v1/search",
                Some("q=Hello%20World"),
                "/v1/search?q=Hello%20World",
            ),
            // Origin removal
            ("https://api.example.com/v1/resource", None, "/v1/resource"),
        ];

        for (input_path, input_query, expected) in cases {
            // Note: In real integration, we pass full URL, but here we test the canonicalizer logic.
            // The kernel logic handles origin stripping.

            // Implementation is linked; test validates canonicalize_request_target behavior.
            assert_eq!(
                canonicalize_request_target(input_path, input_query),
                expected,
                "Failed for input: {} ? {:?}",
                input_path,
                input_query
            );
        }
    }

    #[test]
    fn test_idempotency_query_order_conflict_guard() {
        // REQ: query順序差のみでは衝突とせず、本文差異時のみ 409

        // Case A: Query A="a=1&b=2"
        // Case B: Query B="b=2&a=1"

        let target_a = canonicalize_request_target("/path", Some("a=1&b=2"));
        let target_b = canonicalize_request_target("/path", Some("b=2&a=1"));

        // 1. Canonicalization Contract: Order is normalized
        assert_eq!(
            target_a, target_b,
            "Query order difference must be normalized"
        );

        // 2. Conflict Guard Contract Verification
        // Implementation MUST check body hash if targets match.
        let body_a_hash = "hash_of_body_A";
        let body_b_hash = "hash_of_body_A"; // Same body
        let body_c_hash = "hash_of_body_C"; // Diff body

        // Scenario 1: Same Target, Same Body -> Idempotent Replay (OK)
        let is_conflict_1 =
            check_idempotency_conflict(&target_a, body_a_hash, &target_b, body_b_hash);
        assert!(!is_conflict_1, "Same body should not conflict");

        // Scenario 2: Same Target, Diff Body -> Conflict (409)
        let is_conflict_2 =
            check_idempotency_conflict(&target_a, body_a_hash, &target_b, body_c_hash);
        assert!(
            is_conflict_2,
            "Different body with same canonical target MUST conflict"
        );

        // Case C: Empty segments difference
        // "a=1&&b=2" vs "a=1&b=2"
        let target_c = canonicalize_request_target("/path", Some("a=1&&b=2"));
        let target_d = canonicalize_request_target("/path", Some("a=1&b=2"));

        // Assert that they are DIFFERENT (to prevent collision)
        // If they are different, the Conflict Guard (Hash Check) is skipped (or it's a new key).
        assert_ne!(
            target_c, target_d,
            "Empty segments must be preserved to avoid collision"
        );
    }
}
