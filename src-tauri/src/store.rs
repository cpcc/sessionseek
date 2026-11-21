//! 自有索引库(app_data_dir/index.db):sessions + messages 两张表。
//! 扫描线程持有一个写连接,前端命令各自开只读连接(WAL 允许并行)。

use crate::model::{Message, SessionMeta, Source};
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

pub fn open(path: &Path) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    create_schema(&conn)?;
    Ok(conn)
}

/// 查询用轻量连接:不跑 schema,只设超时。
pub fn open_read(path: &Path) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

fn create_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id            TEXT PRIMARY KEY,
            source        TEXT NOT NULL,
            project       TEXT,
            title         TEXT,
            started_at    INTEGER,
            updated_at    INTEGER,
            message_count INTEGER NOT NULL DEFAULT 0,
            source_path   TEXT NOT NULL,
            file_size     INTEGER NOT NULL DEFAULT 0,
            file_mtime    INTEGER NOT NULL DEFAULT 0,
            is_sidechain  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS sessions_updated_idx ON sessions(updated_at DESC);
        CREATE INDEX IF NOT EXISTS sessions_source_idx  ON sessions(source);

        CREATE TABLE IF NOT EXISTS messages (
            session_id   TEXT NOT NULL,
            seq          INTEGER NOT NULL,
            role         TEXT NOT NULL,
            timestamp    INTEGER,
            payload_json TEXT NOT NULL,
            PRIMARY KEY (session_id, seq)
        ) WITHOUT ROWID;

        -- M2:trigram 全文索引(≥3 字符子串匹配,中英文通用)
        CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            text,
            session_id UNINDEXED,
            seq UNINDEXED,
            tokenize = 'trigram'
        );

        -- M2:会话级 token/调用统计
        CREATE TABLE IF NOT EXISTS session_stats (
            session_id        TEXT PRIMARY KEY,
            model             TEXT,
            input_tokens      INTEGER NOT NULL DEFAULT 0,
            output_tokens     INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens  INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            tool_calls        INTEGER NOT NULL DEFAULT 0,
            api_calls         INTEGER NOT NULL DEFAULT 0
        );

        -- M2:schema 版本(v2 引入 fts+stats,升级时清空强制全量重扫)
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )
}

/// 升级迁移:返回当前 schema 版本。v1→v2 时清空旧索引数据,由随后的全量扫描重建。
pub fn migrate(conn: &Connection) -> Result<i64, rusqlite::Error> {
    let version: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    if version < 2 {
        conn.execute("DELETE FROM messages", [])?;
        conn.execute("DELETE FROM sessions", [])?;
        conn.execute("DELETE FROM messages_fts", [])?;
        conn.execute("DELETE FROM session_stats", [])?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', '2')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        log::info!("migrated index schema to v2 (full rescan required)");
    }
    Ok(2)
}

/// 消息可搜索文本:text + thinking + 工具名/输入/输出。
pub fn parts_search_text(parts: &[crate::model::Part]) -> String {
    use crate::model::Part;
    let mut out = String::new();
    const CAP: usize = 60_000; // 单消息索引文本上限,防止个别巨型消息撑爆 fts
    for p in parts {
        if out.len() >= CAP {
            break;
        }
        match p {
            Part::Text { text } | Part::Thinking { text } => {
                out.push_str(text);
                out.push('\n');
            }
            Part::ToolCall { name, input_json, output, .. } => {
                out.push_str(name);
                out.push('\n');
                out.push_str(input_json);
                out.push('\n');
                if let Some(o) = output {
                    out.push_str(o);
                    out.push('\n');
                }
            }
            Part::Meta { raw } => {
                out.push_str(raw);
                out.push('\n');
            }
        }
    }
    out
}

/// 增量水位判断:同 id 且 (file_size, file_mtime) 完全一致则视为未变。
pub fn session_unchanged(
    conn: &Connection,
    id: &str,
    file_size: i64,
    file_mtime: i64,
) -> Result<bool, rusqlite::Error> {
    let ok = conn
        .query_row(
            "SELECT 1 FROM sessions WHERE id = ?1 AND file_size = ?2 AND file_mtime = ?3",
            params![id, file_size, file_mtime],
            |_| Ok(()),
        )
        .map(|_| true)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(false),
            e => Err(e),
        })?;
    Ok(ok)
}

/// 写入一个解析完成的会话(元数据 + 全量消息 + FTS + 统计,单事务)。
pub fn save_parsed(
    conn: &Connection,
    meta: &SessionMeta,
    messages: &[Message],
    stats: Option<&crate::model::SessionStats>,
) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        r#"INSERT INTO sessions (id, source, project, title, started_at, updated_at,
                                 message_count, source_path, file_size, file_mtime, is_sidechain)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
           ON CONFLICT(id) DO UPDATE SET
               project = excluded.project,
               title = excluded.title,
               started_at = excluded.started_at,
               updated_at = excluded.updated_at,
               message_count = excluded.message_count,
               source_path = excluded.source_path,
               file_size = excluded.file_size,
               file_mtime = excluded.file_mtime,
               is_sidechain = excluded.is_sidechain"#,
        params![
            meta.id,
            meta.source.as_str(),
            meta.project,
            meta.title,
            meta.started_at,
            meta.updated_at,
            messages.len() as i64,
            meta.source_path,
            meta.file_size,
            meta.file_mtime,
            meta.is_sidechain as i64,
        ],
    )?;
    tx.execute("DELETE FROM messages WHERE session_id = ?1", params![meta.id])?;
    tx.execute("DELETE FROM messages_fts WHERE session_id = ?1", params![meta.id])?;
    for m in messages {
        let payload = serde_json::to_string(&m.parts).expect("parts serialize");
        tx.execute(
            "INSERT INTO messages (session_id, seq, role, timestamp, payload_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![meta.id, m.seq, role_str(m.role), m.timestamp, payload],
        )?;
        let text = parts_search_text(&m.parts);
        if !text.trim().is_empty() {
            tx.execute(
                "INSERT INTO messages_fts (session_id, seq, text) VALUES (?1, ?2, ?3)",
                params![meta.id, m.seq, text],
            )?;
        }
    }
    if let Some(st) = stats {
        tx.execute(
            r#"INSERT INTO session_stats (session_id, model, input_tokens, output_tokens,
                                          reasoning_tokens, cache_read_tokens, cache_write_tokens,
                                          tool_calls, api_calls)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
               ON CONFLICT(session_id) DO UPDATE SET
                   model = excluded.model,
                   input_tokens = excluded.input_tokens,
                   output_tokens = excluded.output_tokens,
                   reasoning_tokens = excluded.reasoning_tokens,
                   cache_read_tokens = excluded.cache_read_tokens,
                   cache_write_tokens = excluded.cache_write_tokens,
                   tool_calls = excluded.tool_calls,
                   api_calls = excluded.api_calls"#,
            params![
                meta.id,
                st.model,
                st.input_tokens,
                st.output_tokens,
                st.reasoning_tokens,
                st.cache_read_tokens,
                st.cache_write_tokens,
                st.tool_calls,
                st.api_calls,
            ],
        )?;
    } else {
        tx.execute("DELETE FROM session_stats WHERE session_id = ?1", params![meta.id])?;
    }
    tx.commit()
}

fn role_str(r: crate::model::Role) -> &'static str {
    match r {
        crate::model::Role::User => "user",
        crate::model::Role::Assistant => "assistant",
        crate::model::Role::System => "system",
    }
}

fn parse_role(s: &str) -> crate::model::Role {
    match s {
        "user" => crate::model::Role::User,
        "assistant" => crate::model::Role::Assistant,
        _ => crate::model::Role::System,
    }
}

fn row_to_meta(row: &rusqlite::Row) -> rusqlite::Result<SessionMeta> {
    let source: String = row.get("source")?;
    Ok(SessionMeta {
        id: row.get("id")?,
        source: match source.as_str() {
            "codex" => Source::Codex,
            "zcode" => Source::Zcode,
            "dsh" => Source::Dsh,
            _ => Source::Claude,
        },
        project: row.get("project")?,
        title: row.get("title")?,
        started_at: row.get("started_at")?,
        updated_at: row.get("updated_at")?,
        message_count: row.get("message_count")?,
        source_path: row.get("source_path")?,
        file_size: row.get("file_size")?,
        file_mtime: row.get("file_mtime")?,
        is_sidechain: row.get::<_, i64>("is_sidechain")? != 0,
    })
}

pub struct SessionPage {
    pub items: Vec<SessionMeta>,
    pub total: i64,
}

pub fn list_sessions(
    conn: &Connection,
    source: Option<&str>,
    project: Option<&str>,
    offset: i64,
    limit: i64,
) -> Result<SessionPage, rusqlite::Error> {
    let mut where_clauses: Vec<String> = Vec::new();
    let mut bind_values: Vec<String> = Vec::new();
    if let Some(s) = source {
        where_clauses.push(format!("source = ?{}", bind_values.len() + 1));
        bind_values.push(s.to_string());
    }
    if let Some(p) = project {
        where_clauses.push(format!("project = ?{}", bind_values.len() + 1));
        bind_values.push(p.to_string());
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM sessions {where_sql}"),
        rusqlite::params_from_iter(bind_values.iter()),
        |r| r.get(0),
    )?;

    let limit = limit.clamp(1, 1000);
    let sql = format!(
        "SELECT * FROM sessions {where_sql} ORDER BY updated_at DESC LIMIT {limit} OFFSET {offset}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let items = stmt
        .query_map(rusqlite::params_from_iter(bind_values.iter()), row_to_meta)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SessionPage { items, total })
}

pub struct MessagePage {
    pub items: Vec<Message>,
    pub total: i64,
}

pub fn get_messages(
    conn: &Connection,
    session_id: &str,
    offset: i64,
    limit: i64,
) -> Result<MessagePage, rusqlite::Error> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
        params![session_id],
        |r| r.get(0),
    )?;
    let limit = limit.clamp(1, 1000);
    let mut stmt = conn.prepare(
        "SELECT seq, role, timestamp, payload_json FROM messages
         WHERE session_id = ?1 ORDER BY seq LIMIT ?2 OFFSET ?3",
    )?;
    let items = stmt
        .query_map(params![session_id, limit, offset], |row| {
            let payload: String = row.get("payload_json")?;
            Ok(Message {
                seq: row.get("seq")?,
                role: parse_role(&row.get::<_, String>("role")?),
                timestamp: row.get("timestamp")?,
                parts: serde_json::from_str(&payload).unwrap_or_default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MessagePage { items, total })
}

pub fn get_session_meta(conn: &Connection, id: &str) -> Result<Option<SessionMeta>, rusqlite::Error> {
    let ok = conn
        .query_row("SELECT * FROM sessions WHERE id = ?1", params![id], row_to_meta)
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?;
    Ok(ok)
}

pub fn source_counts(conn: &Connection) -> Result<Vec<(Source, i64)>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT source, COUNT(*) FROM sessions GROUP BY source")?;
    let rows = stmt
        .query_map([], |r| {
            let s: String = r.get(0)?;
            let c: i64 = r.get(1)?;
            let source = match s.as_str() {
                "codex" => Source::Codex,
                "zcode" => Source::Zcode,
                "dsh" => Source::Dsh,
                _ => Source::Claude,
            };
            Ok((source, c))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn project_counts(
    conn: &Connection,
    source: Option<&str>,
) -> Result<Vec<(String, i64)>, rusqlite::Error> {
    let (sql, values): (String, Vec<String>) = match source {
        Some(s) => (
            "SELECT project, COUNT(*) FROM sessions WHERE source = ?1 GROUP BY project ORDER BY COUNT(*) DESC".into(),
            vec![s.to_string()],
        ),
        None => (
            "SELECT project, COUNT(*) FROM sessions GROUP BY project ORDER BY COUNT(*) DESC".into(),
            vec![],
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values.iter()), |r| {
            Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------- M2: 全文搜索

/// trigram 需要 ≥3 字符;查询整体作为短语做子串匹配。短查询返回空结果(提示在命令层给出)。
pub fn search_messages(
    conn: &Connection,
    query: &str,
    source: Option<&str>,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<SearchHit>, rusqlite::Error> {
    if query.chars().count() < 3 {
        return Ok(Vec::new());
    }
    let escaped = query.replace('"', "\"\"");
    let sql = format!(
        r#"SELECT f.session_id   AS session_id,
                  f.seq          AS seq,
                  snippet(messages_fts, 0, '⟦', '⟧', '…', 24) AS snip,
                  m.role         AS role,
                  m.timestamp    AS timestamp,
                  s.title        AS title,
                  s.source       AS source,
                  s.project      AS project
           FROM messages_fts f
           JOIN messages m ON m.session_id = f.session_id AND m.seq = f.seq
           JOIN sessions s ON s.id = f.session_id
           WHERE messages_fts MATCH ?1
             {source_clause}
             {project_clause}
           ORDER BY m.timestamp DESC
           LIMIT ?2"#,
        source_clause = source.map(|_| "AND s.source = ?3").unwrap_or(""),
        project_clause = project.map(|_| "AND s.project = ?4").unwrap_or(""),
    );
    let mut stmt = conn.prepare(&sql)?;
    let q = format!("\"{escaped}\"");
    let hit = |r: &rusqlite::Row| -> rusqlite::Result<SearchHit> {
        let source_s: String = r.get("source")?;
        Ok(SearchHit {
            session_id: r.get("session_id")?,
            seq: r.get("seq")?,
            snippet: r.get("snip")?,
            role: parse_role(&r.get::<_, String>("role")?),
            timestamp: r.get("timestamp")?,
            title: r.get("title")?,
            source: match source_s.as_str() {
                "codex" => Source::Codex,
                "zcode" => Source::Zcode,
                "dsh" => Source::Dsh,
                _ => Source::Claude,
            },
            project: r.get("project")?,
        })
    };
    let rows = if source.is_some() || project.is_some() {
        match (source, project) {
            (Some(s), Some(p)) => stmt
                .query_map(params![q, limit, s, p], hit)?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(s), None) => stmt
                .query_map(params![q, limit, s], hit)?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(p)) => stmt
                .query_map(params![q, limit, p], hit)?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => unreachable!(),
        }
    } else {
        stmt.query_map(params![q, limit], hit)?
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(rows)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub session_id: String,
    pub seq: i64,
    pub snippet: String,
    pub role: crate::model::Role,
    pub timestamp: Option<i64>,
    pub title: Option<String>,
    pub source: Source,
    pub project: Option<String>,
}

// ---------------------------------------------------------------- M2: 统计聚合

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SourceStat {
    pub source: Source,
    pub sessions: i64,
    pub messages: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub tool_calls: i64,
    pub api_calls: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelStat {
    pub model: String,
    pub api_calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DayCount {
    pub day: String,
    pub count: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct OverviewStats {
    pub sessions: i64,
    pub messages: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub tool_calls: i64,
    pub api_calls: i64,
    pub by_source: Vec<SourceStat>,
    pub by_model: Vec<ModelStat>,
    pub by_day: Vec<DayCount>,
}

pub fn overview_stats(conn: &Connection) -> Result<OverviewStats, rusqlite::Error> {
    let mut o = OverviewStats::default();
    let (sessions, messages): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(message_count), 0) FROM sessions",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    o.sessions = sessions;
    o.messages = messages;

    let mut stmt = conn.prepare(
        "SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                COALESCE(SUM(reasoning_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                COALESCE(SUM(tool_calls),0), COALESCE(SUM(api_calls),0)
         FROM session_stats",
    )?;
    let vals = stmt.query_row([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    o.input_tokens = vals.0;
    o.output_tokens = vals.1;
    o.reasoning_tokens = vals.2;
    o.cache_read_tokens = vals.3;
    o.tool_calls = vals.4;
    o.api_calls = vals.5;

    // 按来源
    let mut stmt = conn.prepare(
        "SELECT s.source, COUNT(DISTINCT s.id), COALESCE(SUM(s.message_count),0),
                COALESCE(SUM(x.input_tokens),0), COALESCE(SUM(x.output_tokens),0),
                COALESCE(SUM(x.cache_read_tokens),0), COALESCE(SUM(x.tool_calls),0),
                COALESCE(SUM(x.api_calls),0)
         FROM sessions s
         LEFT JOIN session_stats x ON x.session_id = s.id
         GROUP BY s.source",
    )?;
    o.by_source = stmt
        .query_map([], |r| {
            let src: String = r.get(0)?;
            Ok(SourceStat {
                source: match src.as_str() {
                    "codex" => Source::Codex,
                    "zcode" => Source::Zcode,
                    "dsh" => Source::Dsh,
                    _ => Source::Claude,
                },
                sessions: r.get(1)?,
                messages: r.get(2)?,
                input_tokens: r.get(3)?,
                output_tokens: r.get(4)?,
                cache_read_tokens: r.get(5)?,
                tool_calls: r.get(6)?,
                api_calls: r.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // 按模型
    let mut stmt = conn.prepare(
        "SELECT COALESCE(model, '未知'), COALESCE(SUM(api_calls),0),
                COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                COALESCE(SUM(cache_read_tokens),0)
         FROM session_stats GROUP BY model
         ORDER BY SUM(input_tokens) + SUM(output_tokens) DESC LIMIT 20",
    )?;
    o.by_model = stmt
        .query_map([], |r| {
            Ok(ModelStat {
                model: r.get(0)?,
                api_calls: r.get(1)?,
                input_tokens: r.get(2)?,
                output_tokens: r.get(3)?,
                cache_read_tokens: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // 近 30 天会话数分布(按会话开始日期)
    let now_ms: i64 = chrono::Utc::now().timestamp_millis();
    let since = now_ms - 30 * 24 * 3600 * 1000;
    let mut stmt = conn.prepare(
        "SELECT date(started_at / 1000, 'unixepoch', 'localtime') AS d, COUNT(*)
         FROM sessions
         WHERE started_at IS NOT NULL AND started_at >= ?1
         GROUP BY d ORDER BY d",
    )?;
    o.by_day = stmt
        .query_map(params![since], |r| {
            Ok(DayCount { day: r.get(0)?, count: r.get(1)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Part, Role};

    fn temp_db() -> (tempdir::TempDir, Connection) {
        let dir = tempdir::TempDir::new().unwrap();
        let conn = open(&dir.path().join("index.db")).unwrap();
        (dir, conn)
    }

    // 轻量临时目录(测试专用,避免引入 tempfile 依赖)
    mod tempdir {
        pub struct TempDir(std::path::PathBuf);
        impl TempDir {
            pub fn new() -> std::io::Result<TempDir> {
                let base = std::env::temp_dir().join(format!("ss-test-{}", std::process::id()));
                std::fs::create_dir_all(&base)?;
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                let p = base.join(format!("db-{nanos}"));
                std::fs::create_dir_all(&p)?;
                Ok(TempDir(p))
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn roundtrip() {
        let (_dir, conn) = temp_db();
        let meta = SessionMeta {
            id: "claude:abc".into(),
            source: Source::Claude,
            project: Some("/tmp/proj".into()),
            title: Some("标题".into()),
            started_at: Some(1000),
            updated_at: Some(2000),
            message_count: 0,
            source_path: "/x.jsonl".into(),
            file_size: 1,
            file_mtime: 2,
            is_sidechain: false,
        };
        let msgs = vec![Message {
            seq: 1,
            role: Role::User,
            timestamp: Some(1000),
            parts: vec![Part::Text { text: "你好".into() }, Part::Thinking { text: "想".into() }],
        }];
        let stats = crate::model::SessionStats {
            model: Some("glm-test".into()),
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        };
        assert!(!session_unchanged(&conn, "claude:abc", 1, 2).unwrap());
        save_parsed(&conn, &meta, &msgs, Some(&stats)).unwrap();
        assert!(session_unchanged(&conn, "claude:abc", 1, 2).unwrap());
        assert!(!session_unchanged(&conn, "claude:abc", 9, 2).unwrap());

        let page = list_sessions(&conn, Some("claude"), None, 0, 10).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].title.as_deref(), Some("标题"));
        let msgs2 = get_messages(&conn, "claude:abc", 0, 10).unwrap();
        assert_eq!(msgs2.total, 1);
        assert_eq!(msgs2.items[0].parts.len(), 2);
    }

    #[test]
    fn fts_search_chinese_and_english() {
        let (_dir, conn) = temp_db();
        let mut meta = SessionMeta {
            id: "zcode:t1".into(),
            source: Source::Zcode,
            project: Some("/tmp/z".into()),
            title: Some("采购合同".into()),
            started_at: Some(1_700_000_000_000),
            updated_at: Some(1_700_000_100_000),
            message_count: 0,
            source_path: "/db.sqlite".into(),
            file_size: 0,
            file_mtime: 10,
            is_sidechain: false,
        };
        let msgs = |text: &str, seq: i64| Message {
            seq,
            role: Role::Assistant,
            timestamp: Some(1_700_000_050_000),
            parts: vec![Part::Text { text: text.into() }],
        };
        let s1 = crate::model::SessionStats::default();
        save_parsed(&conn, &meta, &[msgs("采购合同产品智能录入功能开发完成", 1), msgs("fix: download template bug", 2)], Some(&s1)).unwrap();
        meta.id = "claude:t2".into();
        meta.source = Source::Claude;
        meta.file_mtime = 11;
        save_parsed(&conn, &meta, &[msgs("这里完全无关的内容", 1)], Some(&s1)).unwrap();

        // 中文子串(≥3 字)
        let hits = search_messages(&conn, "智能录入", None, None, 10).unwrap();
        assert_eq!(hits.len(), 1, "中文子串应命中");
        assert_eq!(hits[0].session_id, "zcode:t1");
        assert!(hits[0].snippet.contains("智能录入"));
        // 英文子串
        let hits = search_messages(&conn, "template", None, None, 10).unwrap();
        assert_eq!(hits.len(), 1, "英文子串应命中");
        assert!(hits[0].snippet.contains("⟦"));
        // 来源过滤
        let hits = search_messages(&conn, "智能录入", Some("claude"), None, 10).unwrap();
        assert_eq!(hits.len(), 0, "来源过滤应生效");
        // 少于 3 字符返回空(命令层负责提示)
        assert!(search_messages(&conn, "你好", None, None, 10)
            .unwrap()
            .is_empty());

        // 统计聚合
        let ov = overview_stats(&conn).unwrap();
        assert_eq!(ov.sessions, 2);
        assert_eq!(ov.messages, 3);
        assert_eq!(ov.by_source.len(), 2);
    }
}
