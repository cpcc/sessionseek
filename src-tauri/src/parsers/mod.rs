//! 四家 harness 解析器。每个解析器负责:
//! 1. discover() 发现候选会话(增量水位)
//! 2. parse_*() 把单个会话解析为归一化模型

pub mod claude;
pub mod codex;
pub mod dsh;
pub mod zcode;

use crate::model::{Message, SessionMeta, Source};
use std::path::{Path, PathBuf};

/// 扫描候选:待处理会话的定位信息 + 增量水位。
pub struct Candidate {
    /// `{source}:{local_id}`
    pub id: String,
    pub source: Source,
    pub source_path: PathBuf,
    pub file_size: i64,
    pub file_mtime: i64,
}

/// 解析产物。
pub struct ParsedSession {
    pub meta: SessionMeta,
    pub messages: Vec<Message>,
}

impl ParsedSession {
    pub fn new(candidate: &Candidate) -> Self {
        ParsedSession {
            meta: SessionMeta {
                id: candidate.id.clone(),
                source: candidate.source,
                project: None,
                title: None,
                started_at: None,
                updated_at: None,
                message_count: 0,
                source_path: candidate.source_path.display().to_string(),
                file_size: candidate.file_size,
                file_mtime: candidate.file_mtime,
                is_sidechain: false,
            },
            messages: Vec::new(),
        }
    }

    pub fn finish(&mut self) {
        self.meta.message_count = self.messages.len() as i64;
        // updated_at 兜底取最后一条消息时间
        if self.meta.updated_at.is_none() {
            self.meta.updated_at = self.messages.last().and_then(|m| m.timestamp);
        }
        // 标题兜底:首条用户 Text
        if self.meta.title.is_none() {
            'outer: for m in &self.messages {
                if m.role != crate::model::Role::User {
                    continue;
                }
                for p in &m.parts {
                    if let crate::model::Part::Text { text } = p {
                        if !text.trim().is_empty() {
                            self.meta.title = Some(truncate_chars(text, 80));
                            break 'outer;
                        }
                    }
                }
            }
        }
    }
}

/// 时间工具:ISO 8601 → unix 毫秒。
pub fn iso_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let cut: String = s.chars().take(max_chars).collect();
    format!("{cut}\n…(已截断,共 {} 字符)", s.chars().count())
}

/// 工具输出统一截断,避免撑爆索引库。
pub const MAX_TOOL_OUTPUT: usize = 12_000;

pub fn file_stamp(path: &Path) -> (i64, i64) {
    let (size, mtime) = match std::fs::metadata(path) {
        Ok(md) => (
            md.len() as i64,
            md.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        ),
        Err(_) => (0, 0),
    };
    (size, mtime)
}

/// 递归收集匹配扩展名的文件(codex 的日期层级目录)。
pub fn walk_files(root: &Path, depth: usize, pred: &impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if depth == 0 {
        return out;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            out.extend(walk_files(&p, depth - 1, pred));
        } else if pred(&p) {
            out.push(p);
        }
    }
    out
}
