//! Codex:~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl(.zst)。
//! response_item 流线性化;function_call/output 按 call_id 配对;zstd 流式解压。

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
    let root = home.join(".codex").join("sessions");
    let files = super::walk_files(&root, 4, &|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        (name.starts_with("rollout-") && name.ends_with(".jsonl"))
            || (name.starts_with("rollout-") && name.ends_with(".jsonl.zst"))
    });
    for path in files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // local id 去掉扩展名:rollout-2026-06-08T…-<uuid>[.zst]
        let stem = name
            .strip_suffix(".jsonl.zst")
            .or_else(|| name.strip_suffix(".jsonl"))
            .unwrap_or(name);
        let (size, mtime) = super::file_stamp(&path);
        out.push(Candidate {
            id: format!("codex:{stem}"),
            source: Source::Codex,
            source_path: path,
            file_size: size,
            file_mtime: mtime,
        });
    }
    out
}

pub fn parse(candidate: &Candidate) -> std::io::Result<ParsedSession> {
    let file = std::fs::File::open(&candidate.source_path)?;
    let is_zst = candidate
        .source_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.ends_with(".zst"))
        .unwrap_or(false);
    let reader: Box<dyn BufRead> = if is_zst {
        // 流式解压:避免一次性解压大文件占满内存
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

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let ts = v.get("timestamp").and_then(|t| t.as_str()).and_then(iso_to_ms);
        if ps.meta.started_at.is_none() {
            ps.meta.started_at = ts;
        }
        if ts.is_some() {
            ps.meta.updated_at = ts;
        }

        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session_meta" => {
                let payload = v.get("payload");
                if let Some(cwd) = payload.and_then(|p| p.get("cwd")).and_then(|c| c.as_str()) {
                    ps.meta.project = Some(cwd.to_string());
                }
            }
            "turn_context" => {
                // 模型来源(每 turn 一条,取最后)
                if let Some(model) = v.get("payload").and_then(|p| p.get("model")).and_then(|m| m.as_str()) {
                    ps.stats.model = Some(model.to_string());
                }
            }
            "event_msg" => {
                // token_count.info.total_token_usage 为会话累计值,取最大(防 compaction 重置)
                let payload = v.get("payload");
                if payload.and_then(|p| p.get("type")).and_then(|t| t.as_str()) == Some("token_count") {
                    ps.stats.api_calls += 1;
                    if let Some(t) = payload
                        .and_then(|p| p.get("info"))
                        .and_then(|i| i.get("total_token_usage"))
                    {
                        ps.stats.input_tokens = ps.stats.input_tokens.max(
                            t.get("input_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
                        );
                        ps.stats.output_tokens = ps.stats.output_tokens.max(
                            t.get("output_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
                        );
                        ps.stats.reasoning_tokens = ps.stats.reasoning_tokens.max(
                            t.get("reasoning_output_tokens")
                                .and_then(|x| x.as_i64())
                                .unwrap_or(0),
                        );
                        ps.stats.cache_read_tokens = ps.stats.cache_read_tokens.max(
                            t.get("cached_input_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
                        );
                    }
                }
            }
            "response_item" => {
                let Some(payload) = v.get("payload") else { continue };
                let itype = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match itype {
                    "message" => {
                        let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
                        if role != "user" && role != "assistant" {
                            continue; // developer/system 指令噪音,M1 不入库
                        }
                        let msg_role = if role == "user" { Role::User } else { Role::Assistant };
                        let mut msg = crate::model::Message::new(msg_role, ts);
                        if let Some(items) = payload.get("content").and_then(|c| c.as_array()) {
                            for item in items {
                                let ttype = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                if ttype.ends_with("_text") || ttype == "text" {
                                    let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                    if !text.trim().is_empty() {
                                        msg.parts.push(Part::Text { text: text.to_string() });
                                    }
                                }
                            }
                        }
                        if !msg.parts.is_empty() {
                            msg.seq = ps.messages.len() as i64 + 1;
                            let idx = ps.messages.len();
                            ps.messages.push(msg);
                            let _ = idx;
                        }
                    }
                    "function_call" => {
                        let name = payload.get("name").and_then(|n| n.as_str()).unwrap_or("?").to_string();
                        let input_json = payload
                            .get("arguments")
                            .and_then(|a| a.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let call_id = payload.get("call_id").and_then(|c| c.as_str()).unwrap_or("").to_string();
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
                    "function_call_output" => {
                        let call_id = payload.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                        let output = payload
                            .get("output")
                            .map(|o| match o {
                                Value::String(s) => s.clone(),
                                other => serde_json::to_string(other).unwrap_or_default(),
                            })
                            .unwrap_or_default();
                        if let Some(&(mi, pi)) = pending.get(call_id) {
                            if let Part::ToolCall { output: o, status, .. } = &mut ps.messages[mi].parts[pi] {
                                *o = Some(truncate_chars(&output, MAX_TOOL_OUTPUT));
                                *status = "completed".into();
                            }
                        }
                    }
                    "reasoning" => {
                        let summary = payload
                            .get("summary")
                            .and_then(|s| s.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|it| it.get("text").and_then(|t| t.as_str()))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_default();
                        if !summary.trim().is_empty() {
                            let mut msg = crate::model::Message::new(Role::Assistant, ts);
                            msg.parts.push(Part::Thinking { text: summary });
                            msg.seq = ps.messages.len() as i64 + 1;
                            ps.messages.push(msg);
                        }
                    }
                    _ => {}
                }
            }
            // event_msg / turn_context 等纯运行时事件:M1 不入库(避免索引膨胀)
            _ => {}
        }
    }

    ps.finish();
    Ok(ps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture_plain() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/codex/sample.jsonl");
        let cand = Candidate {
            id: "codex:sample".into(),
            source: Source::Codex,
            source_path: path,
            file_size: 1,
            file_mtime: 2,
        };
        let ps = parse(&cand).unwrap();
        assert_eq!(ps.meta.project.as_deref(), Some("/Users/ckj/dev/app/oncameraspeaking"));
        let tool = ps.messages.iter().find_map(|m| {
            m.parts.iter().find_map(|p| match p {
                Part::ToolCall { name, output, status, .. } if name == "exec_command" => {
                    Some((output.clone(), status.clone()))
                }
                _ => None,
            })
        });
        let (output, status) = tool.expect("exec_command tool call");
        assert!(output.unwrap().contains("Process exited"));
        assert_eq!(status, "completed");
        assert!(ps.messages.iter().any(|m| m.role == Role::User));
    }

    #[test]
    fn parse_fixture_zst() {
        // 运行时压缩 fixture,验证 .zst 路径
        let plain = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/codex/sample.jsonl");
        let raw = std::fs::read(&plain).unwrap();
        let dir = std::env::temp_dir().join(format!("ss-zst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let zst_path = dir.join("rollout-sample.jsonl.zst");
        std::fs::write(&zst_path, zstd::stream::encode_all(&raw[..], 3).unwrap()).unwrap();
        let cand = Candidate {
            id: "codex:sample-zst".into(),
            source: Source::Codex,
            source_path: zst_path.clone(),
            file_size: 1,
            file_mtime: 2,
        };
        let ps = parse(&cand).unwrap();
        assert!(!ps.messages.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
