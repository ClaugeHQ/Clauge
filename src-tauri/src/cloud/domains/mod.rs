pub mod agent;
pub mod coworkers;
pub mod explorer;
pub mod nosql;
pub mod rest;
pub mod sql;
pub mod ssh;
pub mod util;
pub mod workspace_notes;

pub const ALL_KINDS: &[&str] = &[
    rest::KIND,
    sql::KIND,
    nosql::KIND,
    agent::KIND,
    ssh::KIND,
    explorer::KIND,
    coworkers::KIND,
    workspace_notes::KIND,
];

/// Build the (hash, base64-gzip-json) tuple for a kind.
pub async fn export_kind(
    pool: &sqlx::SqlitePool,
    kind: &str,
) -> Result<(String, String), String> {
    match kind {
        rest::KIND => rest::export(pool).await,
        sql::KIND => sql::export(pool).await,
        nosql::KIND => nosql::export(pool).await,
        agent::KIND => agent::export(pool).await,
        ssh::KIND => ssh::export(pool).await,
        explorer::KIND => explorer::export(pool).await,
        coworkers::KIND => coworkers::export(pool).await,
        workspace_notes::KIND => workspace_notes::export(pool).await,
        _ => Err(format!("unknown kind: {}", kind)),
    }
}

pub async fn import_kind(
    pool: &sqlx::SqlitePool,
    kind: &str,
    payload_b64: &str,
) -> Result<(), String> {
    let payload = util::decode(payload_b64)?;
    if payload.kind != kind {
        return Err(format!(
            "payload kind mismatch: header says {}, route says {}",
            payload.kind, kind
        ));
    }
    match kind {
        rest::KIND => rest::import(pool, &payload).await,
        sql::KIND => sql::import(pool, &payload).await,
        nosql::KIND => nosql::import(pool, &payload).await,
        agent::KIND => agent::import(pool, &payload).await,
        ssh::KIND => ssh::import(pool, &payload).await,
        explorer::KIND => explorer::import(pool, &payload).await,
        coworkers::KIND => coworkers::import(pool, &payload).await,
        workspace_notes::KIND => workspace_notes::import(pool, &payload).await,
        _ => Err(format!("unknown kind: {}", kind)),
    }
}

#[allow(dead_code)] // called by merge orchestration (Task 3.3)
pub async fn merge_kind(
    pool: &sqlx::SqlitePool,
    kind: &str,
    payload_b64: &str,
) -> Result<(), String> {
    let payload = util::decode(payload_b64)?;
    if payload.kind != kind {
        return Err(format!(
            "payload kind mismatch: header says {}, route says {}",
            payload.kind, kind
        ));
    }
    let specs = match kind {
        rest::KIND => rest::merge_specs(),
        sql::KIND => sql::merge_specs(),
        nosql::KIND => nosql::merge_specs(),
        agent::KIND => agent::merge_specs(),
        ssh::KIND => ssh::merge_specs(),
        explorer::KIND => explorer::merge_specs(),
        coworkers::KIND => coworkers::merge_specs(),
        workspace_notes::KIND => workspace_notes::merge_specs(),
        _ => return Err(format!("unknown kind: {}", kind)),
    };
    util::merge_import(pool, &payload, specs).await
}

#[cfg(test)]
mod spec_tests {
    #[test]
    fn merge_specs_are_internally_consistent() {
        let all: &[(&str, &[crate::cloud::domains::util::TableSpec])] = &[
            ("rest", crate::cloud::domains::rest::merge_specs()),
            ("sql", crate::cloud::domains::sql::merge_specs()),
            ("nosql", crate::cloud::domains::nosql::merge_specs()),
            ("agent", crate::cloud::domains::agent::merge_specs()),
            ("ssh", crate::cloud::domains::ssh::merge_specs()),
            ("explorer", crate::cloud::domains::explorer::merge_specs()),
            ("coworkers", crate::cloud::domains::coworkers::merge_specs()),
            ("workspace_notes", crate::cloud::domains::workspace_notes::merge_specs()),
        ];
        for (kind, specs) in all {
            assert!(!specs.is_empty(), "{kind} has no specs");
            for s in *specs {
                assert!(s.columns.contains(&s.pk), "{kind}/{} missing pk col", s.table);
                if let Some(u) = s.updated_at {
                    assert!(s.columns.contains(&u), "{kind}/{} missing updated_at col", s.table);
                }
            }
        }
    }
}
