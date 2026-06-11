// Shared upstream-error mapping for the AI clients (streaming and
// collect paths). One mapper so the user-facing strings stay identical
// regardless of which path hit the provider.

use crate::shared::ai::context::truncate_str;

/// Pull the rate-limit detail headers off a response BEFORE its body is
/// consumed, so `map_upstream_error` can enrich the 429 message.
pub fn rate_limit_detail(headers: &reqwest::header::HeaderMap) -> (Option<f64>, Option<String>) {
    let retry_after = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok());
    let remaining_tokens = headers
        .get("x-ratelimit-remaining-tokens")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    (retry_after, remaining_tokens)
}

/// Map an upstream non-2xx into the user-facing error string.
///
/// 402 always produces a message containing "credits" so frontend error
/// mappers classify it without parsing JSON. Clauge AI returns:
///   {"error":"INSUFFICIENT_CREDITS","message":"out of Clauge AI credits this cycle","retryable":false}
/// while other providers may return a generic OpenAI-shape body.
pub fn map_upstream_error(
    status: u16,
    error_body: &str,
    retry_after_secs: Option<f64>,
    remaining_tokens: Option<&str>,
) -> String {
    match status {
        401 => "Invalid API key".to_string(),
        402 => {
            let detail = serde_json::from_str::<serde_json::Value>(error_body)
                .ok()
                .and_then(|v| {
                    v["message"]
                        .as_str()
                        .or_else(|| v["error"]["message"].as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_default();
            if detail.is_empty() {
                "Out of credits — payment required".to_string()
            } else if detail.to_lowercase().contains("credits") {
                detail
            } else {
                format!("Out of credits — {}", detail)
            }
        }
        429 => {
            let mut m = "Rate limited".to_string();
            if let Some(secs) = retry_after_secs {
                m.push_str(&format!(" — retry in {:.0}s", secs));
            } else {
                m.push_str(" — try again in a moment");
            }
            if let Some(rem) = remaining_tokens {
                m.push_str(&format!(" ({} tokens remaining)", rem));
            }
            m
        }
        _ => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(error_body) {
                // OpenAI format: {"error": {"message": "..."}}
                if let Some(msg) = parsed["error"]["message"].as_str() {
                    msg.to_string()
                }
                // Mistral format: {"message": {"detail": [{"msg": "..."}]}}
                else if let Some(detail) = parsed["message"]["detail"].as_array() {
                    detail
                        .iter()
                        .filter_map(|d| d["msg"].as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                }
                // Mistral format: {"message": "string"}
                else if let Some(msg) = parsed["message"].as_str() {
                    msg.to_string()
                } else {
                    format!("API error ({}): {}", status, truncate_str(error_body, 200))
                }
            } else {
                format!("API error ({}): {}", status, truncate_str(error_body, 200))
            }
        }
    }
}
