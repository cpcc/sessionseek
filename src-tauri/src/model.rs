//! 归一化模型:四家 harness 的会话统一映射到 SessionMeta / Message / Part。

use serde::{Deserialize, Serialize};

/// 会话来源。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Claude,
    Codex,
    Zcode,
    Dsh,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Claude => "claude",
            Source::Codex => "codex",
            Source::Zcode => "zcode",
            Source::Dsh => "dsh",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Source::Claude => "Claude Code",
            Source::Codex => "Codex",
            Source::Zcode => "ZCode",
            Source::Dsh => "DeepSeek Harness",
        }
    }
}

/// 消息角色。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

/// 消息内的内容块。`Meta` 兜底存放无法识别但值得保留的原样 JSON。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Part {
    Text { text: String },
    Thinking { text: String },
    ToolCall {
        name: String,
        input_json: String,
        output: Option<String>,
        status: String,
    },
    Meta { raw: String },
}

/// 单条消息。parts 序列化后作为 payload_json 存入索引库。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub seq: i64,
    pub role: Role,
    pub timestamp: Option<i64>,
    pub parts: Vec<Part>,
}

impl Message {
    pub fn new(role: Role, timestamp: Option<i64>) -> Self {
        Message {
            seq: 0,
            role,
            timestamp,
            parts: Vec::new(),
        }
    }
}

/// 会话元数据(索引库 sessions 行的内存映像)。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    /// 全局唯一 id:`{source}:{来源内 id}`
    pub id: String,
    pub source: Source,
    pub project: Option<String>,
    pub title: Option<String>,
    /// unix 毫秒
    pub started_at: Option<i64>,
    /// unix 毫秒
    pub updated_at: Option<i64>,
    pub message_count: i64,
    pub source_path: String,
    /// 增量扫描水位:文件大小(文件型来源);zcode 恒为 0
    pub file_size: i64,
    /// 增量扫描水位:mtime 秒;对 zcode 存 time_updated 毫秒
    pub file_mtime: i64,
    pub is_sidechain: bool,
}
