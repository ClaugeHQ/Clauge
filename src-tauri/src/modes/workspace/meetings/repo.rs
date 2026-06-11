use sqlx::SqlitePool;

use crate::modes::workspace::models::{TranscriptSegment, WorkspaceMeeting};

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn json_err(e: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(e))
}

pub async fn insert_meeting(
    pool: &SqlitePool,
    title: &str,
    source_app: Option<&str>,
    language: &str,
) -> Result<WorkspaceMeeting, sqlx::Error> {
    let id = new_id();
    let now = now_rfc3339();
    sqlx::query(
        "INSERT INTO workspace_meetings \
         (id, title, source_app, started_at, language, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 'recording', ?, ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(source_app)
    .bind(&now)
    .bind(language)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    get_meeting(pool, &id).await
}

pub async fn list_meetings(pool: &SqlitePool) -> Result<Vec<WorkspaceMeeting>, sqlx::Error> {
    sqlx::query_as::<_, WorkspaceMeeting>(
        "SELECT * FROM workspace_meetings ORDER BY started_at DESC",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_meeting(pool: &SqlitePool, id: &str) -> Result<WorkspaceMeeting, sqlx::Error> {
    sqlx::query_as::<_, WorkspaceMeeting>("SELECT * FROM workspace_meetings WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

pub async fn update_title(pool: &SqlitePool, id: &str, title: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE workspace_meetings SET title = ?, updated_at = ? WHERE id = ?")
        .bind(title)
        .bind(now_rfc3339())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// AI generation passes `provider`/`model` and stamps
/// `notes_generated_at`; manual edits pass `None` and leave the
/// provenance columns untouched.
pub async fn update_notes(
    pool: &SqlitePool,
    id: &str,
    notes_md: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = now_rfc3339();
    if provider.is_some() {
        sqlx::query(
            "UPDATE workspace_meetings \
             SET notes_md = ?, notes_provider = ?, notes_model = ?, \
                 notes_generated_at = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(notes_md)
        .bind(provider)
        .bind(model)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("UPDATE workspace_meetings SET notes_md = ?, updated_at = ? WHERE id = ?")
            .bind(notes_md)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Read-modify-write on the transcript JSON. Assumes a single writer
/// per meeting (one recorder flush task); concurrent appends to the
/// same meeting would lose segments.
pub async fn append_segments(
    pool: &SqlitePool,
    id: &str,
    segments: &[TranscriptSegment],
) -> Result<(), sqlx::Error> {
    let transcript: String =
        sqlx::query_scalar("SELECT transcript FROM workspace_meetings WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await?;
    let mut all: Vec<TranscriptSegment> =
        serde_json::from_str(&transcript).map_err(json_err)?;
    all.extend_from_slice(segments);
    let json = serde_json::to_string(&all).map_err(json_err)?;
    sqlx::query("UPDATE workspace_meetings SET transcript = ?, updated_at = ? WHERE id = ?")
        .bind(json)
        .bind(now_rfc3339())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn finish_meeting(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    let now = now_rfc3339();
    sqlx::query(
        "UPDATE workspace_meetings \
         SET ended_at = ?, status = 'transcribed', updated_at = ? \
         WHERE id = ?",
    )
    .bind(&now)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_meeting(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM workspace_meetings WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        crate::db::migrator::MIGRATOR
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    fn seg(start_ms: u64, end_ms: u64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start_ms,
            end_ms,
            source: "mic".into(),
            text: text.into(),
        }
    }

    fn parsed(m: &WorkspaceMeeting) -> Vec<TranscriptSegment> {
        serde_json::from_str(&m.transcript).expect("transcript json")
    }

    #[tokio::test]
    async fn meeting_insert_defaults() {
        let pool = test_pool().await;
        let m = insert_meeting(&pool, "Standup", Some("zoom"), "auto")
            .await
            .unwrap();
        assert_eq!(m.title, "Standup");
        assert_eq!(m.source_app.as_deref(), Some("zoom"));
        assert_eq!(m.language, "auto");
        assert_eq!(m.status, "recording");
        assert_eq!(m.transcript, "[]");
        assert!(m.ended_at.is_none());
        assert!(m.workspace_id.is_none());
        assert!(!m.started_at.is_empty());
    }

    #[tokio::test]
    async fn meeting_append_finish_notes_delete() {
        let pool = test_pool().await;
        let m = insert_meeting(&pool, "Design sync", None, "en")
            .await
            .unwrap();

        append_segments(&pool, &m.id, &[seg(0, 1000, "hello")])
            .await
            .unwrap();
        append_segments(
            &pool,
            &m.id,
            &[seg(1000, 2000, "from"), seg(2000, 3000, "clauge")],
        )
        .await
        .unwrap();

        let got = get_meeting(&pool, &m.id).await.unwrap();
        let segs = parsed(&got);
        assert_eq!(segs.len(), 3);
        assert_eq!(
            segs,
            vec![
                seg(0, 1000, "hello"),
                seg(1000, 2000, "from"),
                seg(2000, 3000, "clauge"),
            ]
        );

        finish_meeting(&pool, &m.id).await.unwrap();
        let got = get_meeting(&pool, &m.id).await.unwrap();
        assert_eq!(got.status, "transcribed");
        assert!(got.ended_at.is_some());

        update_notes(&pool, &m.id, "# Notes", Some("anthropic"), Some("opus"))
            .await
            .unwrap();
        let got = get_meeting(&pool, &m.id).await.unwrap();
        assert_eq!(got.notes_md.as_deref(), Some("# Notes"));
        assert_eq!(got.notes_provider.as_deref(), Some("anthropic"));
        assert_eq!(got.notes_model.as_deref(), Some("opus"));
        assert!(got.notes_generated_at.is_some());

        delete_meeting(&pool, &m.id).await.unwrap();
        assert!(matches!(
            get_meeting(&pool, &m.id).await,
            Err(sqlx::Error::RowNotFound)
        ));
    }

    #[tokio::test]
    async fn meeting_manual_notes_edit_keeps_provenance() {
        let pool = test_pool().await;
        let m = insert_meeting(&pool, "Retro", None, "auto").await.unwrap();

        update_notes(&pool, &m.id, "draft", None, None).await.unwrap();
        let got = get_meeting(&pool, &m.id).await.unwrap();
        assert_eq!(got.notes_md.as_deref(), Some("draft"));
        assert!(got.notes_provider.is_none());
        assert!(got.notes_generated_at.is_none());

        update_notes(&pool, &m.id, "# AI", Some("clauge"), Some("haiku"))
            .await
            .unwrap();
        update_notes(&pool, &m.id, "# AI, edited", None, None)
            .await
            .unwrap();
        let got = get_meeting(&pool, &m.id).await.unwrap();
        assert_eq!(got.notes_md.as_deref(), Some("# AI, edited"));
        assert_eq!(got.notes_provider.as_deref(), Some("clauge"));
        assert_eq!(got.notes_model.as_deref(), Some("haiku"));
        assert!(got.notes_generated_at.is_some());
    }

    #[tokio::test]
    async fn meeting_list_orders_by_started_at_desc() {
        let pool = test_pool().await;
        let a = insert_meeting(&pool, "First", None, "auto").await.unwrap();
        let b = insert_meeting(&pool, "Second", None, "auto").await.unwrap();
        sqlx::query("UPDATE workspace_meetings SET started_at = ? WHERE id = ?")
            .bind("2026-06-10T00:00:00.000Z")
            .bind(&a.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE workspace_meetings SET started_at = ? WHERE id = ?")
            .bind("2026-06-11T00:00:00.000Z")
            .bind(&b.id)
            .execute(&pool)
            .await
            .unwrap();

        let list = list_meetings(&pool).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, b.id);
        assert_eq!(list[1].id, a.id);
    }

    async fn fts_ids(pool: &SqlitePool, query: &str) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT meeting_id FROM workspace_meetings_fts WHERE workspace_meetings_fts MATCH ?",
        )
        .bind(query)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn meeting_fts_indexes_transcript_text_not_json() {
        let pool = test_pool().await;
        let m = insert_meeting(&pool, "Sync", None, "en").await.unwrap();
        append_segments(
            &pool,
            &m.id,
            &[seg(0, 1000, "the xylophone budget"), seg(1000, 2000, "is approved")],
        )
        .await
        .unwrap();

        assert_eq!(fts_ids(&pool, "xylophone").await, vec![m.id.clone()]);
        assert!(fts_ids(&pool, "startms").await.is_empty());
        assert!(fts_ids(&pool, "source").await.is_empty());
        assert!(fts_ids(&pool, "mic").await.is_empty());

        delete_meeting(&pool, &m.id).await.unwrap();
        assert!(fts_ids(&pool, "xylophone").await.is_empty());
    }

    #[tokio::test]
    async fn meeting_update_title() {
        let pool = test_pool().await;
        let m = insert_meeting(&pool, "Untitled", None, "auto").await.unwrap();
        update_title(&pool, &m.id, "Q3 planning").await.unwrap();
        let got = get_meeting(&pool, &m.id).await.unwrap();
        assert_eq!(got.title, "Q3 planning");
    }
}
