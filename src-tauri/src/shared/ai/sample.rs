// Bounded model-facing samples.
//
// Background: every mode used to return only metadata to the model ("N rows
// shown in panel", "Status 200 response in result card") which left the
// model blind to what its own tools just produced. It would then re-probe
// or guess — wasting paid tool rounds. These helpers return small samples
// the model can reason about, capped so a wide row or huge response can't
// blow the context window.
//
// The user still sees the full result in the dedicated UI panel; the
// sample is purely the model-facing side of the tool result string.

/// Build a JSON-array sample of objects, one per row, keyed by column
/// name. Caps at `max_rows` and `max_bytes` (whichever hits first).
pub fn format_row_sample(
    columns: &[String],
    rows: &[Vec<serde_json::Value>],
    max_rows: usize,
    max_bytes: usize,
) -> String {
    let take = rows.len().min(max_rows);
    let sample: Vec<serde_json::Value> = rows
        .iter()
        .take(take)
        .map(|r| {
            let obj: serde_json::Map<String, serde_json::Value> = columns
                .iter()
                .zip(r.iter())
                .map(|(c, v)| (c.clone(), v.clone()))
                .collect();
            serde_json::Value::Object(obj)
        })
        .collect();
    let mut body = serde_json::to_string_pretty(&sample).unwrap_or_else(|_| "[]".to_string());
    if body.len() > max_bytes {
        body.truncate(max_bytes);
        body.push_str("\n… (sample truncated)");
    }
    body
}

/// Build a JSON-array sample from already-object documents (Mongo find /
/// aggregate produces these directly — no separate column vector needed).
pub fn format_doc_sample(
    docs: &[serde_json::Value],
    max_docs: usize,
    max_bytes: usize,
) -> String {
    let take = docs.len().min(max_docs);
    let sample: Vec<&serde_json::Value> = docs.iter().take(take).collect();
    let mut body = serde_json::to_string_pretty(&sample).unwrap_or_else(|_| "[]".to_string());
    if body.len() > max_bytes {
        body.truncate(max_bytes);
        body.push_str("\n… (sample truncated)");
    }
    body
}

/// Trim a free-form text payload (HTTP body, file contents) to a byte cap
/// with an inline truncation marker. UTF-8 safe: truncates at the last
/// char boundary at or below `max_bytes` so a multi-byte char isn't cut.
pub fn format_text_sample(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n… (sample truncated)", &text[..cut])
}
