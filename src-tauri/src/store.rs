//! 自有索引库(app_data_dir/index.db):sessions + messages 两张表。
//! 扫描线程持有一个写连接,前端命令各自开只读连接(WAL 允许并行)。

use crate::model::{Message, SessionMeta, Source};
use rusqlite::{params, Connection, OpenFlags};
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
        "#,
    )
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

/// 写入一个解析完成的会话(元数据 + 全量消息,单事务)。
pub fn save_parsed(conn: &Connection, meta: &SessionMeta, messages: &[Message]) -> Result<(), rusqlite::Error> {
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
    for m in messages {
        let payload = serde_json::to_string(&m.parts).expect("parts serialize");
        tx.execute(
            "INSERT INTO messages (session_id, seq, role, timestamp, payload_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![meta.id, m.seq, role_str(m.role), m.timestamp, payload],
        )?;
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
        assert!(!session_unchanged(&conn, "claude:abc", 1, 2).unwrap());
        save_parsed(&conn, &meta, &msgs).unwrap();
        assert!(session_unchanged(&conn, "claude:abc", 1, 2).unwrap());
        assert!(!session_unchanged(&conn, "claude:abc", 9, 2).unwrap());

        let page = list_sessions(&conn, Some("claude"), None, 0, 10).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].title.as_deref(), Some("标题"));
        let msgs2 = get_messages(&conn, "claude:abc", 0, 10).unwrap();
        assert_eq!(msgs2.total, 1);
        assert_eq!(msgs2.items[0].parts.len(), 2);
    }
}
