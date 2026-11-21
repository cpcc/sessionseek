//! ZCode:~/.zcode/cli/db/db.sqlite(SQLite)。只读打开,WAL 撞锁时回退为快照拷贝。
//! 按 session.time_updated 做每会话增量水位。

use super::{truncate_chars, Candidate, ParsedSession, MAX_TOOL_OUTPUT};
use crate::model::{Part, Role, Source};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

pub fn db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".zcode").join("cli").join("db").join("db.sqlite"))
}

/// 打开源库。ZCode 运行中持有 WAL,普通只读连接偶发撞锁 → 回退为临时快照。
pub fn open_source(path: &PathBuf) -> Result<Connection, rusqlite::Error> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    if let Ok(c) = Connection::open_with_flags(path, flags) {
        c.busy_timeout(Duration::from_secs(5))?;
        if probe(&c).is_ok() {
            return Ok(c);
        }
        let _ = c.close();
    }
    open_snapshot(path)
}

fn probe(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.query_row("SELECT COUNT(*) FROM session", [], |r| r.get::<_, i64>(0))
        .map(|_| ())
}

fn open_snapshot(path: &PathBuf) -> Result<Connection, rusqlite::Error> {
    let tmp = std::env::temp_dir().join(format!("ss-zcode-snap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    for suffix in ["", "-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{}", path.display(), suffix));
        if src.exists() {
            let dst = tmp.join(format!("snap.sqlite{}", suffix));
            std::fs::copy(&src, &dst)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        }
    }
    let conn = Connection::open_with_flags(tmp.join("snap.sqlite"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

pub struct ZcodeCandidate {
    pub local_id: String,
    pub title: Option<String>,
    pub directory: Option<String>,
    pub time_created: Option<i64>,
    pub time_updated: i64,
}

pub fn list_sessions(conn: &Connection) -> Result<Vec<ZcodeCandidate>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, title, directory, time_created, time_updated
         FROM session WHERE time_deleted IS NULL",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ZcodeCandidate {
                local_id: r.get(0)?,
                title: r.get(1)?,
                directory: r.get(2)?,
                time_created: r.get(3)?,
                time_updated: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Candidate 构造:水位 = time_updated(毫秒),file_size 恒 0。
pub fn make_candidate(c: &ZcodeCandidate, db: &PathBuf) -> Candidate {
    Candidate {
        id: format!("zcode:{}", c.local_id),
        source: Source::Zcode,
        source_path: db.clone(),
        file_size: 0,
        file_mtime: c.time_updated,
    }
}

pub fn parse(conn: &Connection, cand: &ZcodeCandidate, candidate: &Candidate) -> Result<ParsedSession, rusqlite::Error> {
    let mut ps = ParsedSession::new(candidate);
    ps.meta.title = cand.title.clone();
    ps.meta.project = cand.directory.clone();
    ps.meta.started_at = cand.time_created;
    ps.meta.updated_at = Some(cand.time_updated);

    let mut msg_stmt = conn.prepare(
        "SELECT id, data, time_created, sequence FROM message
         WHERE session_id = ?1 ORDER BY COALESCE(sequence, 2147483647), time_created, id",
    )?;
    let msg_rows: Vec<(String, String, i64)> = msg_stmt
        .query_map([&cand.local_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(msg_stmt);

    let mut part_stmt = conn.prepare(
        "SELECT data FROM part WHERE message_id = ?1
         ORDER BY COALESCE(sequence, 2147483647), time_created, id",
    )?;

    for (msg_id, data, time_created) in msg_rows {
        let Ok(data) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        let role = match data.get("role").and_then(|r| r.as_str()) {
            Some("assistant") => Role::Assistant,
            Some("user") => Role::User,
            _ => Role::System,
        };
        let ts = data
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(|t| t.as_i64())
            .or(Some(time_created));
        let mut msg = crate::model::Message::new(role, ts);

        let parts = part_stmt
            .query_map([&msg_id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for pdata in parts {
            let Ok(p) = serde_json::from_str::<Value>(&pdata) else {
                continue;
            };
            match p.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    let text = p.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    if !text.trim().is_empty() {
                        msg.parts.push(Part::Text { text: text.to_string() });
                    }
                }
                Some("reasoning") => {
                    let text = p.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    if !text.trim().is_empty() {
                        msg.parts.push(Part::Thinking { text: text.to_string() });
                    }
                }
                Some("tool") => {
                    let name = p.get("tool").and_then(|t| t.as_str()).unwrap_or("?").to_string();
                    let (input_json, output, status) = match p.get("state") {
                        Some(state) => {
                            let input = state
                                .get("input")
                                .map(|i| serde_json::to_string(i).unwrap_or_default())
                                .unwrap_or_default();
                            let output = state
                                .get("output")
                                .and_then(|o| o.as_str())
                                .map(|s| truncate_chars(s, MAX_TOOL_OUTPUT));
                            let status = state
                                .get("status")
                                .and_then(|s| s.as_str())
                                .unwrap_or("running")
                                .to_string();
                            (input, output, status)
                        }
                        None => (String::new(), None, "running".into()),
                    };
                    msg.parts.push(Part::ToolCall { name, input_json, output, status });
                }
                // file(图片 artifact)/step-start/step-finish/timeline/compaction:M1 跳过
                _ => {}
            }
        }

        if !msg.parts.is_empty() {
            msg.seq = ps.messages.len() as i64 + 1;
            ps.messages.push(msg);
        }
    }

    ps.finish();
    Ok(ps)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用内存 SQLite 复刻源库 schema,验证消息拼装逻辑。
    #[test]
    fn parse_from_synthetic_db() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE session (id text primary key, title text, directory text,
                time_created integer, time_updated integer, time_deleted integer);
            CREATE TABLE message (id text primary key, session_id text, data text,
                time_created integer, sequence integer);
            CREATE TABLE part (id text primary key, message_id text, data text,
                time_created integer, sequence integer);
            INSERT INTO session VALUES ('sess1', '测试会话', '/tmp/z', 1000, 2000, NULL);
            INSERT INTO message VALUES ('m1', 'sess1',
                '{"role":"user","time":{"created":1000}}', 1000, 0);
            INSERT INTO part VALUES ('p1', 'm1', '{"type":"text","text":"你好"}', 1000, 0);
            INSERT INTO message VALUES ('m2', 'sess1',
                '{"role":"assistant","time":{"created":1100,"completed":1200}}', 1100, 1);
            INSERT INTO part VALUES ('p2', 'm2', '{"type":"reasoning","text":"思考中"}', 1100, 0);
            INSERT INTO part VALUES ('p3', 'm2', '{"type":"tool","tool":"Bash",
                "state":{"status":"completed","input":{"command":"ls"},"output":"a\nb"}}', 1150, 1);
            "#,
        )
        .unwrap();

        let zcand = ZcodeCandidate {
            local_id: "sess1".into(),
            title: Some("测试会话".into()),
            directory: Some("/tmp/z".into()),
            time_created: Some(1000),
            time_updated: 2000,
        };
        let cand = make_candidate(&zcand, &PathBuf::from("/tmp/db.sqlite"));
        let ps = parse(&conn, &zcand, &cand).unwrap();
        assert_eq!(ps.messages.len(), 2);
        assert_eq!(ps.meta.title.as_deref(), Some("测试会话"));
        let tool = ps.messages[1].parts.iter().find_map(|p| match p {
            Part::ToolCall { name, output, status, .. } => Some((name.clone(), output.clone(), status.clone())),
            _ => None,
        });
        let (name, output, status) = tool.expect("tool part");
        assert_eq!(name, "Bash");
        assert!(output.unwrap().contains('a'));
        assert_eq!(status, "completed");
    }
}
