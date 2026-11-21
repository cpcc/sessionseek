//! Claude Code:~/.claude/projects/<munged>/*.jsonl,每文件一个会话。
//! 记录按行线性化(parentUuid 忽略),tool_result 回填到对应 tool_use 块。

use super::{iso_to_ms, truncate_chars, Candidate, ParsedSession, MAX_TOOL_OUTPUT};
use crate::model::{Part, Role, Source};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};

pub fn discover() -> Vec<Candidate> {
    let mut out = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return out;
    };
    let root = home.join(".claude").join("projects");
    let files = super::walk_files(&root, 2, &|p| {
        p.extension().map(|e| e == "jsonl").unwrap_or(false)
    });
    for path in files {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let (size, mtime) = super::file_stamp(&path);
        out.push(Candidate {
            id: format!("claude:{stem}"),
            source: Source::Claude,
            source_path: path,
            file_size: size,
            file_mtime: mtime,
        });
    }
    out
}

pub fn parse(candidate: &Candidate) -> std::io::Result<ParsedSession> {
    let file = std::fs::File::open(&candidate.source_path)?;
    let reader = BufReader::new(file);
    let mut ps = ParsedSession::new(candidate);
    // tool_use id → (消息下标, part 下标),tool_result 到达时回填
    let mut pending: HashMap<String, (usize, usize)> = HashMap::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let rtype = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        // 会话级字段
        if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()).and_then(iso_to_ms) {
            if ps.meta.started_at.is_none() {
                ps.meta.started_at = Some(ts);
            }
            ps.meta.updated_at = Some(ts);
        }
        if let Some(cwd) = v.get("cwd").and_then(|c| c.as_str()) {
            ps.meta.project = Some(cwd.to_string());
        }
        if rtype == "summary" {
            if let Some(s) = v.get("summary").and_then(|s| s.as_str()) {
                ps.meta.title = Some(s.to_string());
            }
        }
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
            ps.meta.is_sidechain = true;
        }

        if rtype != "user" && rtype != "assistant" {
            continue;
        }
        let role = if rtype == "user" { Role::User } else { Role::Assistant };
        let ts = v.get("timestamp").and_then(|t| t.as_str()).and_then(iso_to_ms);
        let content = v.get("message").and_then(|m| m.get("content"));
        let Some(content) = content else { continue };

        let msg_idx = ps.messages.len();
        let mut msg = crate::model::Message::new(role, ts);

        match content {
            Value::String(text) => {
                if role == Role::User && !text.trim().is_empty() {
                    msg.parts.push(Part::Text { text: text.clone() });
                }
            }
            Value::Array(blocks) => {
                for block in blocks {
                    let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match btype {
                        "text" => {
                            let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
                            if !text.trim().is_empty() {
                                msg.parts.push(Part::Text { text: text.to_string() });
                            }
                        }
                        "thinking" => {
                            let text = block.get("thinking").and_then(|t| t.as_str()).unwrap_or("");
                            if !text.trim().is_empty() {
                                msg.parts.push(Part::Thinking { text: text.to_string() });
                            }
                        }
                        "tool_use" => {
                            let name = block.get("name").and_then(|t| t.as_str()).unwrap_or("?").to_string();
                            let input_json = block
                                .get("input")
                                .map(|i| serde_json::to_string(i).unwrap_or_default())
                                .unwrap_or_default();
                            let call_id = block.get("id").and_then(|t| t.as_str()).unwrap_or("").to_string();
                            let idx = msg.parts.len();
                            msg.parts.push(Part::ToolCall {
                                name,
                                input_json,
                                output: None,
                                status: "running".into(),
                            });
                            if !call_id.is_empty() {
                                pending.insert(call_id, (msg_idx, idx));
                            }
                        }
                        "tool_result" => {
                            let call_id = block.get("tool_use_id").and_then(|t| t.as_str()).unwrap_or("");
                            let output = extract_tool_result_text(block);
                            let is_error = block.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false);
                            if let Some(&(mi, pi)) = pending.get(call_id) {
                                if let Part::ToolCall { output: o, status, .. } = &mut ps.messages[mi].parts[pi] {
                                    *o = Some(output);
                                    *status = if is_error { "error" } else { "completed" }.into();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        if !msg.parts.is_empty() {
            msg.seq = ps.messages.len() as i64 + 1;
            ps.messages.push(msg);
        }
    }

    ps.finish();
    Ok(ps)
}

/// tool_result.content:字符串或 [{type:text,text}] 数组,拼接为纯文本。
fn extract_tool_result_text(block: &Value) -> String {
    let text = match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|it| {
                if it.get("type").and_then(|t| t.as_str()) == Some("text") {
                    it.get("text").and_then(|t| t.as_str()).map(truncate_small)
                } else if it.get("type").and_then(|t| t.as_str()) == Some("image") {
                    Some("[图片]".into())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    truncate_chars(&text, MAX_TOOL_OUTPUT)
}

fn truncate_small(s: &str) -> String {
    s.chars().take(2000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude/sample.jsonl");
        let (size, mtime) = (123, 456);
        let cand = Candidate {
            id: "claude:sample".into(),
            source: Source::Claude,
            source_path: path,
            file_size: size,
            file_mtime: mtime,
        };
        let ps = parse(&cand).unwrap();
        assert!(ps.messages.len() >= 3);
        assert_eq!(ps.meta.id, "claude:sample");
        assert_eq!(ps.meta.file_size, 123);
        // 工具调用应被回填输出
        let filled = ps.messages.iter().find_map(|m| {
            m.parts.iter().find_map(|p| match p {
                Part::ToolCall { name, output, status, .. } if name == "Bash" => {
                    Some((output.clone(), status.clone()))
                }
                _ => None,
            })
        });
        let (output, status) = filled.expect("Bash tool call should exist");
        assert!(output.unwrap().contains("laojie"));
        assert_eq!(status, "completed");
        // 标题来自 summary 或首个用户消息
        assert!(ps.meta.title.is_some());
    }
}
