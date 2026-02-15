use url::Url;

/// REQ-IDEMPOTENCY-HEADER: Canonical Request Target logic
/// - `canonical_path`: originを除いたパスを用い、`/` 以外の末尾スラッシュのみ削除する。小文字化・URLデコードは行ってはならない。
/// - `canonical_query`: クエリパラメータはキー昇順、同一キーは値昇順で並べ替える。空クエリは省略し、重複キーは保持する。
/// - `canonical_request_target`: `canonical_path` と `canonical_query` を `?` で連結して生成する（クエリが空なら `canonical_path` のみ）。
pub fn canonicalize_request_target(path: &str, query: Option<&str>) -> String {
    // 1. Path Canonicalization
    // If path contains origin, verify we only take the path part.
    // However, the contract says "origin removed path". The input `path` here corresponds to what the server receives.
    // If it's a full URL, we parse it. If it's just a path, we treat it as path.

    let path_str = if path.starts_with("http://") || path.starts_with("https://") {
        if let Ok(parsed) = Url::parse(path) {
            parsed.path().to_string()
        } else {
            // Fallback or Error? Contract implies we should handle it.
            // If parse fails, treat as raw string? Let's assume valid URL if scheme is present.
            path.to_string()
        }
    } else {
        path.to_string()
    };

    // Remove trailing slash if it's not root "/"
    let canonical_path = if path_str.len() > 1 && path_str.ends_with('/') {
        &path_str[..path_str.len() - 1]
    } else {
        &path_str
    };

    // 2. Query Canonicalization
    let canonical_query = if let Some(q) = query {
        if q.is_empty() {
            None
        } else {
            // Split by '&' to get raw parameters
            let mut pairs: Vec<&str> = q.split('&').collect();

            // Sort raw parameters
            // This is a simple lexicographical sort.
            // It satisfies "Key asc, then Value asc" effectively for raw strings,
            // while preserving the exact encoding sent by the client.
            pairs.sort();

            // Reconstruct query string
            // "空クエリは省略し、重複キーは保持する"
            // We do NOT filter empty parts to ensure `a=1&&b=2` != `a=1&b=2`.
            // Empty parts (resulting from &&) are treated as empty parameters.

            Some(pairs.join("&"))
        }
    } else {
        None
    };

    // 3. Concat
    match canonical_query {
        Some(q) => format!("{}?{}", canonical_path, q),
        None => canonical_path.to_string(),
    }
}

/// REQ-IDEMPOTENCY-CONFLICT: Conflict Guard logic
/// Returns true if a conflict is detected (Same Key, Different Body).
pub fn check_idempotency_conflict(
    canonical_key_a: &str,
    body_hash_a: &str,
    canonical_key_b: &str,
    body_hash_b: &str,
) -> bool {
    if canonical_key_a == canonical_key_b {
        // Same Key: Body Hash MUST match for Idempotency.
        // If diff, it's a conflict.
        body_hash_a != body_hash_b
    } else {
        // Different Keys: No conflict (independent requests)
        false
    }
}
