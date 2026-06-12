// Structured meeting-notes generation from a stored transcript.
//
// Pure pieces (rendering, chunking, fence stripping) are plain functions
// with unit tests. Orchestration routes through the shared AI plumbing:
// the provider registry + `clients::collect::collect_completion` for
// BYOK, and the same path with the user's cloud bearer token (Pro-gated
// via `ProStateManager`) for Clauge AI.

use std::collections::HashMap;

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

use crate::modes::workspace::meetings::repo;
use crate::modes::workspace::models::TranscriptSegment;
use crate::shared::ai::clients::collect::{collect_completion, CollectParams};
use crate::shared::repos::ai_usage as ai_usage_repo;
use crate::shared::repos::settings as settings_repo;

pub const MAX_CHUNK_CHARS: usize = 24_000;
pub const OVERLAP_CHARS: usize = 1_500;

const EVT_PROGRESS: &str = "meetings:notes-progress";
const EVT_READY: &str = "meetings:notes-ready";
const EVT_ERROR: &str = "meetings:notes-error";

/// Usage-attribution mode label (ai_usage table + Clauge AI worker log).
const USAGE_MODE: &str = "meetings";

/// Frontend slug for the managed provider in the meetings UI; maps onto
/// the registry's `clauge` entry.
const CLAUGE_AI_SLUG: &str = "clauge-ai";

macro_rules! notes_format_spec {
    () => {
        "Output ONLY GitHub-flavored markdown — no preamble, no code fences, no commentary.\n\
Use exactly these sections, in this order:\n\
# <concise descriptive meeting title>\n\
## Summary\n3-6 sentences covering the meeting's purpose and outcome.\n\
## Attendees\nBullet list of participants — include this section ONLY if names or roles are clearly inferable.\n\
## Key Decisions\nBullet list of decisions actually made.\n\
## Action Items\nCheckbox list, one line per item: `- [ ] **owner** — item`. Use **Unassigned** as the owner when none is stated.\n\
## Deadlines\nBullet list pairing each date or timeframe with what is due.\n\
## Next Steps\nBullet list of agreed follow-ups.\n\
Omit any section (heading included) that would be empty. Never invent facts, names, dates, or commitments that are not supported by the transcript."
    };
}

pub const NOTES_SYSTEM_PROMPT: &str = concat!(
    "You are an expert meeting-notes writer. You turn raw meeting transcripts into crisp, \
     skimmable notes.\n\
     The transcript is machine speech-recognition output: silently fix obvious \
     mis-transcriptions from context, but never invent facts. Speaker labels: [mic] is the \
     note-taker (refer to them as \"you\"), [system] is the other participants.\n\n",
    notes_format_spec!()
);

pub const CHUNK_DIGEST_PROMPT: &str = "You are given one PORTION of a longer meeting transcript \
(machine speech-recognition output; [mic] is the note-taker, [system] is the other \
participants — silently fix obvious mis-transcriptions, never invent facts).\n\
Extract terse factual bullets ONLY: topics discussed, decisions made, action items with their \
owners, deadlines, and names mentioned. One fact per bullet, prefixed with `- `. No prose, no \
headings, no interpretation. If this portion contains nothing noteworthy, output `- (nothing \
noteworthy)`.";

pub const MERGE_PROMPT: &str = concat!(
    "You are an expert meeting-notes writer. You are given ordered factual digests of \
     consecutive portions of ONE meeting, in meeting order. Consecutive portions overlap \
     slightly, so deduplicate repeated facts. Combine the digests into final notes for the \
     whole meeting.\n\n",
    notes_format_spec!()
);

/// One transcript segment per line: `[mm:ss] [mic|system] text`,
/// in timeline order regardless of the order segments were stored.
pub fn render_transcript(segments: &[TranscriptSegment]) -> String {
    let mut ordered: Vec<&TranscriptSegment> = segments.iter().collect();
    ordered.sort_by(|a, b| {
        (a.start_ms, a.end_ms, a.source.as_str()).cmp(&(b.start_ms, b.end_ms, b.source.as_str()))
    });
    let mut out = String::new();
    for seg in ordered {
        let text = seg.text.trim();
        if text.is_empty() {
            continue;
        }
        let secs = seg.start_ms / 1000;
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!(
            "[{:02}:{:02}] [{}] {}",
            secs / 60,
            secs % 60,
            seg.source,
            text
        ));
    }
    out
}

/// Sliding window over LINES: each chunk is ≤ `max_chars` (unless a
/// single line alone exceeds it), boundaries always fall on line breaks,
/// and consecutive chunks share ~`overlap_chars` of trailing lines.
pub fn split_transcript(rendered: &str, max_chars: usize, overlap_chars: usize) -> Vec<String> {
    let lines: Vec<&str> = rendered.lines().collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0usize;

    while start < lines.len() {
        let mut end = start;
        let mut size = 0usize;
        while end < lines.len() {
            let add = lines[end].len() + 1;
            if size + add > max_chars && end > start {
                break;
            }
            size += add;
            end += 1;
        }
        chunks.push(lines[start..end].join("\n"));
        if end >= lines.len() {
            break;
        }
        let mut overlap = 0usize;
        let mut next_start = end;
        while next_start > start + 1 && overlap + lines[next_start - 1].len() + 1 <= overlap_chars
        {
            overlap += lines[next_start - 1].len() + 1;
            next_start -= 1;
        }
        start = next_start;
    }
    chunks
}

/// Models sometimes wrap their whole markdown answer in a code fence
/// despite instructions. Unwrap only when the entire output is fenced.
pub fn strip_code_fences(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        if let Some(end) = rest.rfind("```") {
            let inner = &rest[..end];
            let inner = match inner.find('\n') {
                Some(i) => &inner[i + 1..],
                None => inner,
            };
            return inner.trim().to_string();
        }
    }
    t.to_string()
}

pub async fn generate_notes(
    app: &AppHandle,
    pool: &SqlitePool,
    meeting_id: &str,
    provider_id: &str,
    model: Option<&str>,
) -> Result<String, String> {
    let result = generate_notes_inner(app, pool, meeting_id, provider_id, model).await;
    // Failure counterpart of EVT_READY: the run may have outlived the tab
    // that invoked it, so the command rejection alone can't reach a
    // reopened meeting view — the event can.
    if let Err(message) = &result {
        let _ = app.emit(
            EVT_ERROR,
            serde_json::json!({"meetingId": meeting_id, "message": message}),
        );
    }
    result
}

async fn generate_notes_inner(
    app: &AppHandle,
    pool: &SqlitePool,
    meeting_id: &str,
    provider_id: &str,
    model: Option<&str>,
) -> Result<String, String> {
    let meeting = repo::get_meeting(pool, meeting_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Meeting not found".to_string())?;

    let segments: Vec<TranscriptSegment> =
        serde_json::from_str(&meeting.transcript).map_err(|e| format!("Bad transcript: {}", e))?;
    let rendered = render_transcript(&segments);
    if rendered.trim().is_empty() {
        return Err("transcript is empty".to_string());
    }

    let is_cloud = provider_id == CLAUGE_AI_SLUG || provider_id == "clauge";
    let slug = if is_cloud { "clauge" } else { provider_id };
    let config = crate::shared::ai::resolve_config(slug, model)?;

    let auth_state = app.state::<crate::cloud::auth::AuthState>();
    let mut extra_headers: HashMap<String, String> = HashMap::new();
    let (api_key, auth_for_call) = if is_cloud {
        let pro = app.state::<crate::cloud::pro_state::ProStateManager>();
        if !pro.is_pro() {
            return Err("pro_required".to_string());
        }
        let (token, provider) = auth_state
            .active_token_and_provider()
            .ok_or_else(|| "pro_required".to_string())?;
        extra_headers.insert("X-Provider".to_string(), provider);
        (token, Some(auth_state.inner()))
    } else {
        let key = settings_repo::get_by_key(pool, config.key_setting_name)
            .await
            .ok()
            .flatten()
            .map(|s| s.value)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| "no_api_key".to_string())?;
        (key, None)
    };

    let client = crate::shared::http::build_ai_oneshot_http_client(pool).await?;
    let session_id = format!("meeting-notes-{}", meeting_id);
    let complete = |system: &'static str, user: String| {
        let client = &client;
        let extra_headers = &extra_headers;
        let api_key = api_key.as_str();
        let session_id = session_id.as_str();
        async move {
            let result = collect_completion(
                client,
                app,
                pool,
                api_key,
                config,
                &CollectParams {
                    system,
                    user: &user,
                    mode: USAGE_MODE,
                    session_id,
                    extra_headers,
                    auth_state: auth_for_call,
                },
            )
            .await?;
            let _ = ai_usage_repo::record(
                pool,
                &uuid::Uuid::new_v4().to_string(),
                USAGE_MODE,
                config.model_id,
                result.input_tokens as i64,
                result.output_tokens as i64,
                0,
            )
            .await;
            Ok::<String, String>(result.text)
        }
    };

    let chunks = split_transcript(&rendered, MAX_CHUNK_CHARS, OVERLAP_CHARS);
    // A 0/total progress event fires for EVERY run (single-chunk included)
    // before the first model call, so a meeting tab reopened mid-flight
    // re-raises its in-flight spinner even when no chunk boundary will
    // ever emit progress.
    let started_total = if chunks.len() == 1 { 1 } else { chunks.len() + 1 };
    let _ = app.emit(
        EVT_PROGRESS,
        serde_json::json!({"meetingId": meeting_id, "done": 0, "total": started_total}),
    );
    let notes = if chunks.len() == 1 {
        complete(NOTES_SYSTEM_PROMPT, rendered).await?
    } else {
        let total = chunks.len() + 1;
        let mut digests: Vec<String> = Vec::with_capacity(chunks.len());
        for (i, chunk) in chunks.into_iter().enumerate() {
            digests.push(complete(CHUNK_DIGEST_PROMPT, chunk).await?);
            let _ = app.emit(
                EVT_PROGRESS,
                serde_json::json!({"meetingId": meeting_id, "done": i + 1, "total": total}),
            );
        }
        let n = digests.len();
        let merged_input = digests
            .into_iter()
            .enumerate()
            .map(|(i, d)| format!("Portion {} of {}:\n{}", i + 1, n, d))
            .collect::<Vec<_>>()
            .join("\n\n");
        let merged = complete(MERGE_PROMPT, merged_input).await?;
        let _ = app.emit(
            EVT_PROGRESS,
            serde_json::json!({"meetingId": meeting_id, "done": total, "total": total}),
        );
        merged
    };

    let notes = strip_code_fences(&notes);
    let rows = repo::update_notes(pool, meeting_id, &notes, Some(slug), Some(config.model_id))
        .await
        .map_err(|e| e.to_string())?;
    if rows == 0 {
        return Err("Meeting not found".to_string());
    }
    repo::set_status(pool, meeting_id, "notes_ready")
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit(EVT_READY, serde_json::json!({"meetingId": meeting_id}));
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_ms: u64, source: &str, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start_ms,
            end_ms: start_ms + 1000,
            source: source.into(),
            text: text.into(),
        }
    }

    #[test]
    fn render_formats_timestamp_and_source() {
        let rendered = render_transcript(&[
            seg(0, "mic", "hello everyone"),
            seg(65_000, "system", "hi there"),
            seg(3_725_000, "mic", "  wrapping up  "),
            seg(4_000_000, "mic", "   "),
        ]);
        assert_eq!(
            rendered,
            "[00:00] [mic] hello everyone\n[01:05] [system] hi there\n[62:05] [mic] wrapping up"
        );
    }

    #[test]
    fn render_sorts_interleaved_sources_chronologically() {
        let rendered = render_transcript(&[
            seg(0, "mic", "first"),
            seg(15_000, "mic", "third"),
            seg(5_000, "system", "second"),
            seg(21_000, "mic", "fourth"),
        ]);
        assert_eq!(
            rendered,
            "[00:00] [mic] first\n[00:05] [system] second\n\
             [00:15] [mic] third\n[00:21] [mic] fourth"
        );
    }

    #[test]
    fn short_transcript_is_one_chunk() {
        let rendered = "[00:00] [mic] hello\n[00:01] [system] hi";
        let chunks = split_transcript(rendered, MAX_CHUNK_CHARS, OVERLAP_CHARS);
        assert_eq!(chunks, vec![rendered.to_string()]);
    }

    #[test]
    fn long_transcript_chunks_on_line_breaks_with_overlap_and_no_loss() {
        let lines: Vec<String> = (0..200)
            .map(|i| format!("[{:02}:{:02}] [mic] line number {} with some content", i / 60, i % 60, i))
            .collect();
        let rendered = lines.join("\n");
        let max_chars = 500;
        let overlap_chars = 100;
        let chunks = split_transcript(&rendered, max_chars, overlap_chars);

        assert!(chunks.len() > 1, "expected multiple chunks");
        for chunk in &chunks {
            assert!(chunk.len() <= max_chars, "chunk exceeds max_chars");
            for line in chunk.lines() {
                assert!(
                    lines.iter().any(|l| l == line),
                    "chunk boundary split a line: {:?}",
                    line
                );
            }
        }
        for pair in chunks.windows(2) {
            let prev_last = pair[0].lines().last().unwrap();
            assert!(
                pair[1].lines().any(|l| l == prev_last),
                "no overlap between consecutive chunks"
            );
        }
        for line in &lines {
            assert!(
                chunks.iter().any(|c| c.lines().any(|l| l == line)),
                "line lost during chunking: {:?}",
                line
            );
        }
    }

    #[test]
    fn oversized_single_line_still_progresses() {
        let rendered = format!("{}\n{}", "a".repeat(1000), "b".repeat(1000));
        let chunks = split_transcript(&rendered, 100, 50);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].starts_with('a'));
        assert!(chunks[1].starts_with('b'));
    }

    #[test]
    fn fence_stripping() {
        assert_eq!(strip_code_fences("# Notes\n\n- item"), "# Notes\n\n- item");
        assert_eq!(strip_code_fences("```\n# Notes\n```"), "# Notes");
        assert_eq!(
            strip_code_fences("```markdown\n# Notes\n- item\n```\n"),
            "# Notes\n- item"
        );
        assert_eq!(
            strip_code_fences("# Has\n```sql\nSELECT 1\n```\ninline fences"),
            "# Has\n```sql\nSELECT 1\n```\ninline fences"
        );
    }

    #[test]
    fn prompts_contain_required_sections() {
        for prompt in [NOTES_SYSTEM_PROMPT, MERGE_PROMPT] {
            for section in [
                "## Summary",
                "## Attendees",
                "## Key Decisions",
                "## Action Items",
                "## Deadlines",
                "## Next Steps",
            ] {
                assert!(prompt.contains(section), "missing {}", section);
            }
            assert!(prompt.contains("- [ ] **owner**"));
            assert!(prompt.contains("Unassigned"));
        }
        assert!(CHUNK_DIGEST_PROMPT.contains("PORTION"));
        assert!(NOTES_SYSTEM_PROMPT.contains("[mic]"));
        assert!(NOTES_SYSTEM_PROMPT.contains("[system]"));
    }
}
