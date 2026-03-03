#[cfg(test)]
mod tests {
    use http::StatusCode;
    use kernel_core::quota::{QuotaLayer, QuotaViolation};

    struct QuotaScenario {
        layer: QuotaLayer,
        expected_status: StatusCode,
    }

    #[test]
    fn test_quota_http_matrix() {
        let scenarios = vec![
            QuotaScenario {
                layer: QuotaLayer::TenantBudget,
                expected_status: StatusCode::TOO_MANY_REQUESTS,
            },
            QuotaScenario {
                layer: QuotaLayer::ApiRateLimit,
                expected_status: StatusCode::TOO_MANY_REQUESTS,
            },
            QuotaScenario {
                layer: QuotaLayer::SystemHardLimit,
                expected_status: StatusCode::SERVICE_UNAVAILABLE,
            },
            QuotaScenario {
                layer: QuotaLayer::CircuitBreaker,
                expected_status: StatusCode::SERVICE_UNAVAILABLE,
            },
        ];

        for scenario in scenarios {
            // Mock server behavior check
            // let response = client.trigger_quota(scenario.layer).await;
            // assert_eq!(response.status(), scenario.expected_status);
            // assert!(response.headers().contains_key("Retry-After"));

            // For now, we just document the contract in code.
            let violation = QuotaViolation {
                layer: scenario.layer,
                retry_after_s: 10,
            };
            assert_eq!(violation.status_code(), scenario.expected_status.as_u16());

            let headers = violation.headers();
            let retry_after = headers.iter().find(|(k, _)| k == "Retry-After");
            assert!(retry_after.is_some(), "Retry-After header must be present");
            assert_eq!(retry_after.unwrap().1, "10", "Retry-After value must match");
        }
    }

    #[test]
    fn test_quota_retry_after_boundaries() {
        // Case 1: Zero value (Retry immediately / minimal delay)
        let v_zero = QuotaViolation {
            layer: QuotaLayer::ApiRateLimit,
            retry_after_s: 0,
        };
        let h_zero = v_zero.headers();
        assert_eq!(h_zero[0].1, "0");

        // Case 2: Large value (Generic layer) -> Cap at 1 year
        let v_large = QuotaViolation {
            layer: QuotaLayer::ApiRateLimit,
            retry_after_s: 999_999_999,
        };
        assert_eq!(v_large.headers()[0].1, "31536000");

        // Case 3: SystemHardLimit boundary -> 1-30s clip
        let v_sys_low = QuotaViolation {
            layer: QuotaLayer::SystemHardLimit,
            retry_after_s: 0,
        };
        assert_eq!(v_sys_low.headers()[0].1, "1");

        let v_sys_high = QuotaViolation {
            layer: QuotaLayer::SystemHardLimit,
            retry_after_s: 100,
        };
        assert_eq!(v_sys_high.headers()[0].1, "30");

        let v_sys_ok = QuotaViolation {
            layer: QuotaLayer::SystemHardLimit,
            retry_after_s: 15,
        };
        assert_eq!(v_sys_ok.headers()[0].1, "15");

        // Case 4: CircuitBreaker must floor Retry-After to 1s
        let v_cb_low = QuotaViolation {
            layer: QuotaLayer::CircuitBreaker,
            retry_after_s: 0,
        };
        assert_eq!(v_cb_low.headers()[0].1, "1");

        let v_cb_high = QuotaViolation {
            layer: QuotaLayer::CircuitBreaker,
            retry_after_s: 120,
        };
        assert_eq!(v_cb_high.headers()[0].1, "120");
    }
}
