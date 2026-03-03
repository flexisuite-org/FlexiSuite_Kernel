use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuotaLayer {
    TenantBudget,
    ApiRateLimit,
    SystemHardLimit,
    CircuitBreaker,
}

#[derive(Debug)]
pub struct QuotaViolation {
    pub layer: QuotaLayer,
    // Contract: MUST be present.
    pub retry_after_s: u64,
}

impl QuotaViolation {
    pub fn status_code(&self) -> u16 {
        match self.layer {
            QuotaLayer::TenantBudget | QuotaLayer::ApiRateLimit => 429,
            QuotaLayer::SystemHardLimit | QuotaLayer::CircuitBreaker => 503,
        }
    }

    pub fn headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![];
        // REQ-QUOTA-HTTP-CONTRACT: Must include Retry-After
        let value = match self.layer {
            QuotaLayer::SystemHardLimit => {
                // Spec: 1-30s clip for system protection
                self.retry_after_s.clamp(1, 30)
            }
            QuotaLayer::CircuitBreaker => {
                // Circuit-breaker backoff must never be 0s.
                self.retry_after_s.max(1)
            }
            _ => {
                // Guard: Cap at 1 year (31,536,000s) to prevent overflow/abuse
                self.retry_after_s.min(31_536_000)
            }
        };
        headers.push(("Retry-After".to_string(), value.to_string()));
        headers
    }
}
