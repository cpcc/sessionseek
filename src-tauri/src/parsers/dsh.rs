//! DeepSeek Harness:~/.dsh/sessions/<munged>/<session-dir>/session.vN.jsonl.zstd。
//! vN 多版本并存时取最高版本;tool/call + tool/result 按 callId 配对。

use super::{truncate_chars, Candidate, ParsedSession, MAX_TOOL_OUTPUT};
use crate::model::{Part, Role, Source};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

fn is_session_file(name: &str) -> bool {
    name.starts_with("session.v")
        && (name.ends_with(".jsonl.zstd") || name.ends_with(".jsonl.zst") || name.ends_with(".jsonl"))
}

fn version_of(name: &str) -> u32 {
    // session.v3.jsonl.zstd → 3
    let stem = name
        .trim_end_matches(".jsonl.zstd")
        .trim_end_matches(".jsonl.zst")
        .trim_end_matches(".jsonl");
    stem.strip_prefix("session.v")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0)
}

pub fn discover() -> Vec<Candidate> {
    let mut out = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return out;
    };
    let root = home.join(".dsh").join("sessions");
    let Ok(munged_dirs) = std::fs::read_dir(&root) else {
        return out;
    };
    for mdir in munged_dirs.flatten() {
        let Ok(sess_dirs) = std::fs::read_dir(mdir.path()) else {
            continue;
        };
        for sdir in sess_dirs.flatten() {
            let sdir_path = sdir.path();
            if !sdir_path.is_dir() {
                continue;
            }
            // 取版本号最高的文件作为该会话的当前形态
            let mut best: Option<(u32, PathBuf)> = None;
            if let Ok(files) = std::fs::read_dir(&sdir_path) {
                for f in files.flatten() {
                    let name = f.file_name().to_string_lossy().to_string();
                    if is_session_file(&name) {
                        let v = version_of(&name);
                        if best.as_ref().map(|(bv, _)| v > *bv).unwrap_or(true) {
                            best = Some((v, f.path()));
                        }
                    }
                }
            }
            let Some((_, path)) = best else { continue };
            let Some(dir_name) = sdir_path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let (size, mtime) = super::file_stamp(&path);
            out.push(Candidate {
                id: format!("dsh:{dir_name}"),
                source: Source::Dsh,
                source_path: path,
                file_size: size,
                file_mtime: mtime,
            });
        }
    }
    out
}

pub fn parse(candidate: &Candidate) -> std::io::Result<ParsedSession> {
    let file = std::fs::File::open(&candidate.source_path)?;
    let name = candidate
        .source_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let compressed = name.ends_with(".zstd") || name.ends_with(".zst");
    let reader: Box<dyn BufRead> = if compressed {
        let dec = zstd::stream::Decoder::new(file)?;
        Box::new(BufReader::with_capacity(64 * 1024, dec))
    } else {
        Box::new(BufReader::with_capacity(64 * 1024, file))
    };
    parse_lines(reader, candidate)
}

fn parse_lines(reader: Box<dyn BufRead>, candidate: &Candidate) -> std::io::Result<ParsedSession> {
    let mut ps = ParsedSession::new(candidate);
    let mut pending: HashMap<String, (usize, usize)> = HashMap::new();
    let mut version: u32 = 0;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let etype = v.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
        let ts = v.get("time").and_then(|t| t.as_i64());
        if ps.meta.started_at.is_none() {
            ps.meta.started_at = ts;
        }
        if ts.is_some() {
            ps.meta.updated_at = ts;
        }

        match etype.as_str() {
            "session" => {
                version = v.get("version").and_then(|t| t.as_u64()).unwrap_or(0) as u32;
                if let Some(cwd) = v.get("cwd").and_then(|c| c.as_str()) {
                    ps.meta.project = Some(cwd.to_string());
                }
            }
            "model/selection" => {
                if let Some(m) = v.get("data").and_then(|d| d.get("model")).and_then(|m| m.as_str()) {
                    ps.stats.model = Some(m.to_string());
                }
            }
            "session/title" => {
                if let Some(t) = v.get("data").and_then(|d| d.get("title")).and_then(|t| t.as_str()) {
                    ps.meta.title = Some(t.to_string());
                }
            }
            "user/message" => {
                let mut msg = crate::model::Message::new(Role::User, ts);
                if let Some(items) = v.get("data").and_then(|d| d.get("content")).and_then(|c| c.as_array()) {
                    for item in items {
                        if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                            let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                            if !text.trim().is_empty() {
                                msg.parts.push(Part::Text { text: text.to_string() });
                            }
                        }
                    }
                }
                if !msg.parts.is_empty() {
                    msg.seq = ps.messages.len() as i64 + 1;
                    ps.messages.push(msg);
                }
            }
            "assistant/message" => {
                ps.stats.api_calls += 1;
                // data.usage: {inputTokens, outputTokens, totalTokens, cacheReadTokens}(逐次值)
                if let Some(u) = v.get("data").and_then(|d| d.get("usage")) {
                    ps.stats.input_tokens += u.get("inputTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                    ps.stats.output_tokens += u.get("outputTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                    ps.stats.cache_read_tokens += u.get("cacheReadTokens").and_then(|x| x.as_i64()).unwrap_or(0);
                }
                let mut msg = crate::model::Message::new(Role::Assistant, ts);
                let content = v
                    .get("data")
                    .and_then(|d| d.get("message"))
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array());
                if let Some(items) = content {
                    for item in items {
                        match item.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                if !text.trim().is_empty() {
                                    msg.parts.push(Part::Text { text: text.to_string() });
                                }
                            }
                            Some("reasoning") => {
                                let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                if !text.trim().is_empty() {
                                    msg.parts.push(Part::Thinking { text: text.to_string() });
                                }
                            }
                            // tool-call 内联项与 tool/call 事件重复,去重跳过
                            _ => {}
                        }
                    }
                }
                if !msg.parts.is_empty() {
                    msg.seq = ps.messages.len() as i64 + 1;
                    ps.messages.push(msg);
                }
            }
            "system/message" => {
                let mut msg = crate::model::Message::new(Role::System, ts);
                if let Some(items) = v.get("data").and_then(|d| d.get("content")).and_then(|c| c.as_array()) {
                    for item in items {
                        if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                            let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                            if !text.trim().is_empty() {
                                msg.parts.push(Part::Text { text: text.to_string() });
                            }
                        }
                    }
                }
                if !msg.parts.is_empty() {
                    msg.seq = ps.messages.len() as i64 + 1;
                    ps.messages.push(msg);
                }
            }
            "tool/call" => {
                let data = v.get("data");
                let name = data
                    .and_then(|d| d.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("?")
                    .to_string();
                let input_json = data
                    .and_then(|d| d.get("arguments"))
                    .and_then(|a| a.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let call_id = data
                    .and_then(|d| d.get("callId"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut msg = crate::model::Message::new(Role::Assistant, ts);
                let idx = msg.parts.len();
                msg.parts.push(Part::ToolCall {
                    name,
                    input_json,
                    output: None,
                    status: "running".into(),
                });
                ps.stats.tool_calls += 1;
                msg.seq = ps.messages.len() as i64 + 1;
                if !call_id.is_empty() {
                    pending.insert(call_id, (ps.messages.len(), idx));
                }
                ps.messages.push(msg);
            }
            "tool/result" => {
                let Some(content_items) = v
                    .get("data")
                    .and_then(|d| d.get("message"))
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                else {
                    continue;
                };
                for item in content_items {
                    let itype = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if itype != "tool-result" {
                        continue;
                    }
                    let call_id = item.get("toolCallId").and_then(|c| c.as_str()).unwrap_or("");
                    let output = item
                        .get("content")
                        .and_then(|c| c.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|it| it.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    let is_error = item.get("isError").and_then(|b| b.as_bool()).unwrap_or(false);
                    if let Some(&(mi, pi)) = pending.get(call_id) {
                        if let Part::ToolCall { output: o, status, .. } = &mut ps.messages[mi].parts[pi] {
                            *o = Some(truncate_chars(&output, MAX_TOOL_OUTPUT));
                            *status = if is_error { "error" } else { "completed" }.into();
                        }
                    }
                }
            }
            // turn/*、step/*、llm/*、request/*、permission/* 等运行时事件:M1 不入库
            _ => {}
        }
    }

    // v1/v2 字段形状未实测:若解析出的消息为空但仍存在非 session 事件,标记为 Meta 供排查
    if version > 0 && version < 3 && ps.messages.is_empty() {
        ps.meta.title = Some(format!("(v{version} 格式暂未支持解析)"));
    }
    ps.finish();
    Ok(ps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dsh/sample.jsonl");
        let cand = Candidate {
            id: "dsh:sample".into(),
            source: Source::Dsh,
            source_path: path,
            file_size: 1,
            file_mtime: 2,
        };
        let ps = parse(&cand).unwrap();
        assert_eq!(ps.meta.project.as_deref(), Some("/Users/ckj/dev/sgm/NEW-ERP"));
        assert_eq!(ps.meta.title.as_deref(), Some("询问当前是什么项目")); // 多个 session/title,后者胜出
        // tool/call + tool/result 配对回填
        let tool = ps.messages.iter().find_map(|m| {
            m.parts.iter().find_map(|p| match p {
                Part::ToolCall { name, output, status, .. } if name == "bash" => {
                    Some((output.clone(), status.clone()))
                }
                _ => None,
            })
        });
        let (output, status) = tool.expect("bash tool call");
        assert!(output.unwrap().contains("NEW-ERP"));
        assert_eq!(status, "completed");
        // assistant 内联 tool-call 不应重复产生工具块
        let tool_count: usize = ps
            .messages
            .iter()
            .map(|m| m.parts.iter().filter(|p| matches!(p, Part::ToolCall { .. })).count())
            .sum();
        assert_eq!(tool_count, 1);
    }
}
