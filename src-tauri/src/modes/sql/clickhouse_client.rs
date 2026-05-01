// ClickHouse HTTP client.
//
// ClickHouse is OLAP and HTTP-based, not sqlx-compatible, so this driver
// lives outside the sqlx pool model. It speaks ClickHouse's HTTP interface
// directly (POST /?database=...&user=...&password=...) and uses the
// `FORMAT JSON` response shape, which returns:
//
//   { "meta": [{"name": ..., "type": ...}, ...],
//     "data": [{"col": value, ...}, ...],
//     "rows": N, ... }
//
// Each `ClickhouseClient` is a stateless wrapper over a `reqwest::Client`,
// the base URL, and credentials. Connection-test happens at "connect" time
// by issuing a `SELECT 1`. There is no persistent connection to close.
//
// `reqwest` is already a workspace dependency (REST mode + tauri-plugin-http
// pull it in), so adding ClickHouse here costs zero extra build weight.

use serde::Deserialize;
use std::time::Duration;

use super::client::SqlConnectionConfig;

/// Map ClickHouse error code → short user-facing message.
///
/// Codes are stable across ClickHouse versions (defined in
/// dbms/Common/ErrorCodes.cpp). For unmapped codes, the caller falls back
/// to `clean_clickhouse_error_body` which trims the verbose response body.
fn humanize_clickhouse_code(code: &str) -> Option<&'static str> {
    match code {
        "516" | "192" | "193" => Some("Authentication failed — check username and password"),
        "81" => Some("Database not found — check the database name"),
        "60" => Some("Table not found"),
        "47" => Some("Unknown column"),
        // 62 = SYNTAX_ERROR. Don't humanize — a generic "SQL syntax error"
        // toast hides the actual parser message ("expected SETTINGS",
        // "Cannot parse expression", etc.) which is exactly what the user
        // needs to fix their query. Falls through to clean_clickhouse_error_body.
        "159" => Some("Query timed out"),
        "164" => Some("Read-only — write operation not allowed"),
        "210" => Some("Network error — check host and port"),
        "241" => Some("Memory limit exceeded"),
        "252" => Some("Too many parts — server is overloaded"),
        "394" => Some("Query was cancelled"),
        _ => None,
    }
}

/// Fallback: trim ClickHouse's verbose error body to the salient first
/// sentence + labelled tag (e.g. `(AUTHENTICATION_FAILED)`). Used when
/// `humanize_clickhouse_code` doesn't have a mapping for the error code.
fn clean_clickhouse_error_body(body: &str) -> String {
    let after_exc = body.splitn(2, "DB::Exception:").nth(1).unwrap_or(body);
    let first = after_exc.splitn(2, ". ").next().unwrap_or(after_exc);
    let trimmed = first.trim().trim_end_matches('.').trim();
    let label = body
        .rsplit('(')
        .filter_map(|s| s.split(')').next())
        .find(|s| s.chars().all(|c| c.is_ascii_uppercase() || c == '_') && s.len() > 3);

    let mut out = trimmed.to_string();
    if out.len() > 220 {
        out.truncate(220);
        out.push('…');
    }
    if let Some(l) = label {
        if !out.contains(l) {
            out.push_str(" (");
            out.push_str(l);
            out.push(')');
        }
    }
    out
}

/// Stateless HTTP client for a single ClickHouse connection. Cloning is
/// cheap; `reqwest::Client` shares its connection pool internally.
#[derive(Debug, Clone)]
pub struct ClickhouseClient {
    http: reqwest::Client,
    base_url: String,
    username: String,
    password: String,
    /// Default database — included in the query URL when set.
    pub database: String,
}

/// Wire shape of `FORMAT JSON` responses.
#[derive(Debug, Deserialize)]
struct ClickhouseJsonResponse {
    #[serde(default)]
    meta: Vec<ClickhouseColumnMeta>,
    #[serde(default)]
    data: Vec<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    rows: u64,
}

#[derive(Debug, Deserialize)]
struct ClickhouseColumnMeta {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
}

/// Parsed query result in the same shape `sql_execute_query` returns for
/// the other drivers (column names + JSON-typed row values).
pub struct ClickhouseQueryRows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub affected: u64,
}

impl ClickhouseClient {
    /// Build a client. `host`/`port` come from `SqlConnectionConfig`;
    /// `ssl=true` switches the URL scheme to https.
    pub fn new(config: &SqlConnectionConfig) -> Result<Self, String> {
        let scheme = if config.ssl { "https" } else { "http" };
        let host = if config.host.is_empty() { "localhost".to_string() } else { config.host.clone() };
        let port = if config.port == 0 { 8123 } else { config.port };
        let base_url = format!("{}://{}:{}", scheme, host, port);

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| format!("ClickHouse client build failed: {}", e))?;

        // ClickHouse defaults: user=default, empty password, database=default.
        let username = if config.username.is_empty() { "default".to_string() } else { config.username.clone() };
        let database = if config.database.is_empty() { "default".to_string() } else { config.database.clone() };

        Ok(Self {
            http,
            base_url,
            username,
            password: config.password.clone(),
            database,
        })
    }

    /// Verify reachability + credentials by issuing a `SELECT 1`.
    pub async fn ping(&self) -> Result<(), String> {
        let _ = self.execute_format_json("SELECT 1").await?;
        Ok(())
    }

    /// Execute an arbitrary SQL statement and return parsed rows. The
    /// statement is appended with `FORMAT JSON` when the input doesn't
    /// already specify a FORMAT clause; statements that don't return rows
    /// (e.g. INSERT, ALTER) come back with empty `meta`/`data` and that's
    /// fine.
    pub async fn query(&self, sql: &str) -> Result<ClickhouseQueryRows, String> {
        let resp = self.execute_format_json(sql).await?;
        Ok(map_response_to_rows(resp))
    }

    /// POST the SQL to ClickHouse and parse the response.
    ///
    /// FORMAT-clause policy: Clauge's results panel only renders the
    /// structured `FORMAT JSON` shape (`{ meta: [], data: [], rows: N }`).
    /// We dispatch as follows:
    ///   - No user FORMAT      → append `FORMAT JSON` and parse normally.
    ///   - User wrote `FORMAT JSON` (case-insensitive) → pass through.
    ///   - User wrote any other `FORMAT X` → reject up-front with a clean
    ///     error. Sending it would succeed at the server but our JSON
    ///     parse would then fail with a baffling "JSON parse failed"
    ///     message; the explicit reject is more honest.
    async fn execute_format_json(&self, sql: &str) -> Result<ClickhouseJsonResponse, String> {
        // ClickHouse's HTTP parser is strict about what comes after a FORMAT
        // clause — even a trailing `;` will produce a syntax error. So we
        // strip trailing `;`/whitespace in BOTH branches that send a query
        // to the server. The append-FORMAT-JSON branch additionally uses a
        // single space (not `\n`) as the separator between any preceding
        // clause (e.g. SETTINGS) and the FORMAT keyword — most permissive.
        let trim_trailing = |s: &str| -> String {
            s.trim_end_matches(|c: char| c.is_whitespace() || c == ';')
                .to_string()
        };

        // FORMAT JSON should ONLY be appended to row-returning statements.
        // For INSERT specifically, ClickHouse interprets `FORMAT JSON` as
        // "the data block I'm about to receive is in JSON format" — not
        // "return output as JSON" — and tries to parse the VALUES clause
        // as JSON, producing CANNOT_PARSE_INPUT_ASSERTION_FAILED (code 27).
        // INSERT/ALTER/DROP/UPDATE/DELETE/CREATE/etc. return empty bodies
        // which the `text.trim().is_empty()` branch below handles cleanly.
        let sql_for_send = match extract_user_format(sql) {
            // User wrote FORMAT JSON — pass through (we'd append it anyway
            // for row-returning queries; for non-row ones the user is
            // explicitly telling CH to expect/emit JSON, so honour it).
            Some(ref fmt) if fmt.eq_ignore_ascii_case("JSON") => trim_trailing(sql),
            // User wrote any other FORMAT → reject up-front.
            Some(fmt) => {
                return Err(format!(
                    "Custom FORMAT '{}' is not supported in Clauge's table view. \
                     Remove the FORMAT clause to see results.",
                    fmt
                ));
            }
            // No user FORMAT clause: append `FORMAT JSON` only if the
            // statement returns rows; otherwise send verbatim.
            None if returns_rows(sql) => format!("{} FORMAT JSON", trim_trailing(sql)),
            None => trim_trailing(sql),
        };

        let mut req = self
            .http
            .post(&self.base_url)
            .query(&[("database", self.database.as_str())])
            .header("Content-Type", "text/plain; charset=UTF-8");

        // Send credentials via headers (avoids putting password in URL).
        req = req.header("X-ClickHouse-User", &self.username);
        if !self.password.is_empty() {
            req = req.header("X-ClickHouse-Key", &self.password);
        }

        let response = req
            .body(sql_for_send)
            .send()
            .await
            .map_err(|e| format!("ClickHouse request failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            // ClickHouse returns the error in the body; the
            // X-ClickHouse-Exception-Code header holds the numeric code.
            let code = response
                .headers()
                .get("X-ClickHouse-Exception-Code")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let body = response.text().await.unwrap_or_default();
            // Prefer our hand-written message for known codes; fall back to
            // a trimmed version of ClickHouse's verbose body otherwise.
            return Err(match code.as_deref().and_then(humanize_clickhouse_code) {
                Some(msg) => msg.to_string(),
                None => match code {
                    Some(c) => format!("ClickHouse error (code {}): {}", c, clean_clickhouse_error_body(&body)),
                    None => format!("ClickHouse error ({}): {}", status, clean_clickhouse_error_body(&body)),
                },
            });
        }

        let text = response
            .text()
            .await
            .map_err(|e| format!("ClickHouse response read failed: {}", e))?;

        if text.trim().is_empty() {
            // No-row statements (DDL/INSERT) come back empty.
            return Ok(ClickhouseJsonResponse { meta: vec![], data: vec![], rows: 0 });
        }

        serde_json::from_str::<ClickhouseJsonResponse>(&text)
            .map_err(|e| format!("ClickHouse JSON parse failed: {} (body: {})", e, text.chars().take(400).collect::<String>()))
    }
}

/// Whether `sql` starts with a row-returning ClickHouse statement.
///
/// Row-returning leading keywords (we suffix `FORMAT JSON` for these so
/// the response is parseable structured data):
///   SELECT, WITH (CTE), SHOW, DESCRIBE / DESC, EXPLAIN, CHECK, EXISTS.
///
/// Everything else (INSERT, UPDATE, DELETE, CREATE, ALTER, DROP, RENAME,
/// TRUNCATE, ATTACH, DETACH, OPTIMIZE, SYSTEM, USE, SET, KILL, …) returns
/// an empty body and MUST be sent verbatim. Appending `FORMAT JSON` to
/// an `INSERT … VALUES` is misinterpreted by ClickHouse as "incoming data
/// block is in JSON format" — it then tries to parse the VALUES list as
/// JSON and fails with code 27 (CANNOT_PARSE_INPUT_ASSERTION_FAILED).
fn returns_rows(sql: &str) -> bool {
    let cleaned = strip_sql_comments(sql);
    let trimmed = cleaned.trim_start();
    let first_word: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    matches!(
        first_word.as_str(),
        "SELECT" | "WITH" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" | "CHECK" | "EXISTS"
    )
}

/// Strip SQL comments (line `--` and block `/* */`) while respecting
/// single- and double-quoted string literals. Returns the stripped text;
/// quote-internal `--` / `/*` are preserved verbatim.
///
/// Operates byte-by-byte. SQL keywords/operators are ASCII so this is
/// safe for any UTF-8 input — non-ASCII bytes inside strings/identifiers
/// pass through unchanged.
fn strip_sql_comments(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut i = 0;
    let n = bytes.len();
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while i < n {
        let b = bytes[i];

        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
                out.push('\n');
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if b == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if in_single {
            out.push(b as char);
            // SQL escapes `'` as `''` inside single-quoted strings.
            if b == b'\'' {
                if i + 1 < n && bytes[i + 1] == b'\'' {
                    out.push('\'');
                    i += 2;
                    continue;
                }
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            out.push(b as char);
            if b == b'"' {
                if i + 1 < n && bytes[i + 1] == b'"' {
                    out.push('"');
                    i += 2;
                    continue;
                }
                in_double = false;
            }
            i += 1;
            continue;
        }

        // Outside string + outside comment.
        if b == b'-' && i + 1 < n && bytes[i + 1] == b'-' {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if b == b'\'' {
            in_single = true;
            out.push('\'');
            i += 1;
            continue;
        }
        if b == b'"' {
            in_double = true;
            out.push('"');
            i += 1;
            continue;
        }

        // Regular character — non-ASCII bytes pass through as raw bytes
        // (out is a String so we accumulate via char; multi-byte sequences
        // come through correctly because we only special-case ASCII bytes).
        out.push(b as char);
        i += 1;
    }
    out
}

/// Extract the user-supplied `FORMAT <name>` clause from the trailing
/// position of a ClickHouse SQL statement, returning the format name if
/// present.
///
/// Detection rules:
///   - Comments are stripped first (so `FORMAT JSON` inside `--` / `/* */`
///     does not trigger).
///   - String-literal contents (single or double quoted) are not scanned
///     (so `SELECT 'FORMAT JSON'` does not trigger).
///   - Trailing whitespace and `;` are trimmed.
///   - Match anchors at end-of-string: the LAST identifier in the
///     statement is treated as the format-name candidate. We then walk
///     left over whitespace and verify the previous 6 chars are the
///     `FORMAT` keyword (case-insensitive), with a whitespace boundary
///     before it (so `FORMAT_USED` as a single identifier does not
///     match).
///   - The candidate identifier must start with a letter (format names
///     never start with a digit), so trailing numeric literals (`SETTINGS
///     x = 1`) are not mistaken for formats.
fn extract_user_format(sql: &str) -> Option<String> {
    let cleaned = strip_sql_comments(sql);
    let trimmed = cleaned
        .trim_end_matches(|c: char| c.is_whitespace() || c == ';')
        .trim_end();
    let bytes = trimmed.as_bytes();

    // 1. Walk back over the trailing identifier (alphanumerics + underscore).
    let mut id_start = bytes.len();
    while id_start > 0 {
        let b = bytes[id_start - 1];
        if b.is_ascii_alphanumeric() || b == b'_' {
            id_start -= 1;
        } else {
            break;
        }
    }
    if id_start == bytes.len() {
        return None; // no trailing identifier
    }
    let format_name = std::str::from_utf8(&bytes[id_start..]).ok()?;
    // Format names start with a letter (filters numeric literals).
    if !format_name
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_alphabetic())
    {
        return None;
    }

    // 2. Skip whitespace between the identifier and the FORMAT keyword.
    let mut keyword_end = id_start;
    while keyword_end > 0 && bytes[keyword_end - 1].is_ascii_whitespace() {
        keyword_end -= 1;
    }
    if keyword_end == id_start {
        // No whitespace separator → the identifier IS contiguous with what's
        // before. e.g. `FORMAT_USED` alone, no preceding "FORMAT keyword".
        return None;
    }

    // 3. Verify the 6 chars ending at keyword_end are "FORMAT".
    if keyword_end < 6 {
        return None;
    }
    let keyword = std::str::from_utf8(&bytes[keyword_end - 6..keyword_end]).ok()?;
    if !keyword.eq_ignore_ascii_case("FORMAT") {
        return None;
    }

    // 4. Verify the boundary BEFORE "FORMAT" is whitespace or
    //    start-of-string (rejects e.g. `XFORMAT` adjacent to FORMAT).
    if keyword_end > 6 {
        let prev = bytes[keyword_end - 7];
        if !prev.is_ascii_whitespace() {
            return None;
        }
    }

    Some(format_name.to_string())
}

/// Convert a ClickHouse JSON response into the column/row shape the
/// existing ResultsTable expects. Each column value is mapped from its
/// declared ClickHouse type to a JSON primitive — matching how the
/// Postgres / MySQL / SQLite paths shape their output.
fn map_response_to_rows(resp: ClickhouseJsonResponse) -> ClickhouseQueryRows {
    let columns: Vec<String> = resp.meta.iter().map(|c| c.name.clone()).collect();
    let types: Vec<String> = resp.meta.iter().map(|c| c.type_name.clone()).collect();

    let rows: Vec<Vec<serde_json::Value>> = resp
        .data
        .iter()
        .map(|obj| {
            columns
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let raw = obj.get(name).cloned().unwrap_or(serde_json::Value::Null);
                    let ty = types.get(i).map(|s| s.as_str()).unwrap_or("");
                    coerce_value_for_type(raw, ty)
                })
                .collect()
        })
        .collect();

    let affected = if resp.rows > 0 { resp.rows } else { rows.len() as u64 };
    ClickhouseQueryRows { columns, rows, affected }
}

/// Map a raw ClickHouse JSON cell into the JSON shape the frontend expects.
///
/// ClickHouse's `FORMAT JSON` already produces sensible JSON (numbers stay
/// numbers, strings stay strings, nulls become null). For very large
/// integer types (`UInt64` / `Int64` / 128-bit ints) ClickHouse emits them
/// as JSON strings to avoid f64 precision loss — we keep them as strings,
/// matching how Postgres NUMERIC is surfaced.
fn coerce_value_for_type(raw: serde_json::Value, ty: &str) -> serde_json::Value {
    if matches!(raw, serde_json::Value::Null) {
        return serde_json::Value::Null;
    }
    // Strip Nullable(T) wrapper for type-name dispatch.
    let inner = strip_nullable(ty);

    // Pass-through for the cases ClickHouse already shapes correctly.
    // Arrays/Tuples/Maps come back as JSON arrays/objects; leave them.
    if inner.starts_with("Array(") || inner.starts_with("Tuple(") || inner.starts_with("Map(") {
        return raw;
    }

    // 64-bit ints + Decimal: ClickHouse stringifies these in JSON.
    // Booleans: ClickHouse Bool type → already JSON bool.
    // Everything else we accept as-is.
    let _ = inner; // reserved for future per-type coercion (Date, UUID, etc.)
    raw
}

fn strip_nullable(ty: &str) -> &str {
    if let Some(rest) = ty.strip_prefix("Nullable(") {
        rest.strip_suffix(')').unwrap_or(rest)
    } else {
        ty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_basic_response_to_rows() {
        let resp = ClickhouseJsonResponse {
            meta: vec![
                ClickhouseColumnMeta { name: "id".to_string(), type_name: "UInt32".to_string() },
                ClickhouseColumnMeta { name: "name".to_string(), type_name: "String".to_string() },
                ClickhouseColumnMeta { name: "amount".to_string(), type_name: "Decimal(10, 2)".to_string() },
            ],
            data: vec![
                {
                    let mut m = serde_json::Map::new();
                    m.insert("id".to_string(), serde_json::json!(1));
                    m.insert("name".to_string(), serde_json::json!("alice"));
                    m.insert("amount".to_string(), serde_json::json!("12.50"));
                    m
                },
                {
                    let mut m = serde_json::Map::new();
                    m.insert("id".to_string(), serde_json::json!(2));
                    m.insert("name".to_string(), serde_json::Value::Null);
                    m.insert("amount".to_string(), serde_json::json!("0.00"));
                    m
                },
            ],
            rows: 2,
        };

        let mapped = map_response_to_rows(resp);
        assert_eq!(mapped.columns, vec!["id", "name", "amount"]);
        assert_eq!(mapped.rows.len(), 2);
        assert_eq!(mapped.rows[0][0], serde_json::json!(1));
        assert_eq!(mapped.rows[0][1], serde_json::json!("alice"));
        assert_eq!(mapped.rows[0][2], serde_json::json!("12.50"));
        assert_eq!(mapped.rows[1][1], serde_json::Value::Null);
        assert_eq!(mapped.affected, 2);
    }

    #[test]
    fn nullable_wrapping_is_stripped() {
        assert_eq!(strip_nullable("Nullable(String)"), "String");
        assert_eq!(strip_nullable("String"), "String");
        assert_eq!(strip_nullable("Nullable(DateTime64(3))"), "DateTime64(3)");
    }

    #[test]
    fn extract_user_format_basic() {
        assert_eq!(extract_user_format("SELECT 1 FORMAT JSON"), Some("JSON".into()));
        assert_eq!(extract_user_format("select 1 format TSV"), Some("TSV".into()));
        assert_eq!(extract_user_format("SELECT 1 FORMAT JSONEachRow"), Some("JSONEachRow".into()));
        assert_eq!(extract_user_format("SELECT 1"), None);
    }

    #[test]
    fn extract_user_format_trims_trailing_semicolons_and_whitespace() {
        assert_eq!(extract_user_format("SELECT 1 FORMAT JSON;"), Some("JSON".into()));
        assert_eq!(extract_user_format("SELECT 1 FORMAT JSON ; ;\n"), Some("JSON".into()));
        assert_eq!(extract_user_format("SELECT 1 FORMAT TSV  \n  "), Some("TSV".into()));
    }

    #[test]
    fn extract_user_format_ignores_string_literal_contents() {
        // The single-quoted string literal `'FORMAT JSON'` must not match.
        assert_eq!(extract_user_format("SELECT 'FORMAT JSON' FROM t"), None);
        // Same for double-quoted (treated as identifier in PG/CH).
        assert_eq!(extract_user_format("SELECT \"FORMAT JSON\" FROM t"), None);
        // Real FORMAT clause AFTER a string-literal that mentions format.
        assert_eq!(
            extract_user_format("SELECT 'FORMAT JSON' FROM t FORMAT TSV"),
            Some("TSV".into())
        );
    }

    #[test]
    fn extract_user_format_ignores_comments() {
        // FORMAT JSON inside a line comment must not match; trailing FORMAT TSV does.
        assert_eq!(
            extract_user_format("SELECT 1 -- FORMAT JSON ignored\nFORMAT TSV"),
            Some("TSV".into())
        );
        // Block comment between SELECT and FORMAT.
        assert_eq!(
            extract_user_format("SELECT 1 /* FORMAT JSON */ FORMAT TSV"),
            Some("TSV".into())
        );
        // FORMAT clause entirely inside a block comment.
        assert_eq!(
            extract_user_format("SELECT 1 /* trailing FORMAT TSV */"),
            None
        );
    }

    #[test]
    fn extract_user_format_no_false_positive_for_compound_identifier() {
        // `FORMAT_USED` is a single identifier, not the FORMAT keyword.
        assert_eq!(extract_user_format("SELECT 1 FORMAT_USED"), None);
        // CH SETTINGS with a setting whose name contains "format".
        assert_eq!(
            extract_user_format("SELECT 1 SETTINGS format_csv_delimiter = ','"),
            None
        );
    }

    #[test]
    fn extract_user_format_skips_numeric_trailing() {
        // Trailing numeric literal must not be classified as a format name.
        assert_eq!(extract_user_format("SELECT 1 SETTINGS max_threads = 1"), None);
        assert_eq!(extract_user_format("SELECT 1 LIMIT 100"), None);
    }

    #[test]
    fn returns_rows_classification() {
        // Row-returning leading keywords.
        assert!(returns_rows("SELECT 1"));
        assert!(returns_rows("select * from t"));
        assert!(returns_rows("WITH cte AS (SELECT 1) SELECT * FROM cte"));
        assert!(returns_rows("SHOW TABLES"));
        assert!(returns_rows("DESCRIBE TABLE t"));
        assert!(returns_rows("DESC t"));
        assert!(returns_rows("EXPLAIN SELECT 1"));
        assert!(returns_rows("EXPLAIN PIPELINE SELECT 1"));
        // Comment-tolerance.
        assert!(returns_rows("/* leading */ SELECT 1"));
        assert!(returns_rows("-- line comment\nSELECT 1"));

        // Non-row-returning — the bug case for INSERT was that we appended
        // FORMAT JSON and CH thought VALUES was a JSON data block.
        assert!(!returns_rows("INSERT INTO t VALUES (1)"));
        assert!(!returns_rows("UPDATE t SET x = 1"));
        assert!(!returns_rows("DELETE FROM t"));
        assert!(!returns_rows("CREATE TABLE t (id UInt32) ENGINE = MergeTree"));
        assert!(!returns_rows("ALTER TABLE t ADD COLUMN x UInt32"));
        assert!(!returns_rows("ALTER TABLE t DELETE WHERE x = 1"));
        assert!(!returns_rows("DROP TABLE t"));
        assert!(!returns_rows("OPTIMIZE TABLE t"));
        assert!(!returns_rows("SYSTEM RELOAD CONFIG"));
        assert!(!returns_rows("RENAME TABLE a TO b"));
        assert!(!returns_rows("TRUNCATE TABLE t"));
        assert!(!returns_rows("USE analytics"));
        assert!(!returns_rows(""));
        assert!(!returns_rows("   "));
    }

    #[test]
    fn strip_sql_comments_basic() {
        assert_eq!(strip_sql_comments("SELECT 1 -- comment\n;"), "SELECT 1 \n;");
        assert_eq!(strip_sql_comments("/* leading */ SELECT 1"), " SELECT 1");
        // String literals preserved untouched.
        assert_eq!(strip_sql_comments("SELECT '-- not a comment'"), "SELECT '-- not a comment'");
        assert_eq!(strip_sql_comments("SELECT '/* not block */'"), "SELECT '/* not block */'");
        // SQL-escape '' inside a string literal stays intact.
        assert_eq!(strip_sql_comments("SELECT 'it''s -- fine'"), "SELECT 'it''s -- fine'");
    }
}
