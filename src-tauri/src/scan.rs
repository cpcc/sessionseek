//! 扫描调度:在后台线程运行,发现 → 增量过滤 → 解析 → 写索引,进度事件推给前端。

use crate::model::Source;
use crate::store;
use crate::AppState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub source: String,
    pub done: usize,
    pub total: usize,
    pub updated: usize,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub claude: usize,
    pub codex: usize,
    pub zcode: usize,
    pub dsh: usize,
    pub skipped: usize,
    pub duration_ms: u64,
}

/// 全量扫描入口(setup 启动线程 / scan_now 命令都会走到这里)。
pub fn run_full_scan(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.scanning.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return; // 已有扫描在跑
    }
    let db_path = state.db_path.clone();
    let app = app.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scan_all(&app, &db_path)
        }));
        let summary = match result {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                log::error!("scan failed: {e}");
                ScanSummary { duration_ms: started.elapsed().as_millis() as u64, ..Default::default() }
            }
            Err(_) => {
                log::error!("scan panicked");
                ScanSummary { duration_ms: started.elapsed().as_millis() as u64, ..Default::default() }
            }
        };
        let state = app.state::<AppState>();
        state.scanning.store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = app.emit("scan-finished", &summary);
        log::info!("scan done in {}ms: {:?}", summary.duration_ms, summary);
    });
}

fn scan_all(app: &AppHandle, db_path: &std::path::Path) -> Result<ScanSummary, String> {
    let conn = store::open(db_path).map_err(|e| e.to_string())?;
    store::migrate(&conn).map_err(|e| e.to_string())?;
    let mut summary = ScanSummary::default();
    let started = std::time::Instant::now();

    // ---- 文件型来源:claude / codex / dsh ----
    let file_sources: [(&str, Vec<crate::parsers::Candidate>); 3] = [
        ("claude", crate::parsers::claude::discover()),
        ("codex", crate::parsers::codex::discover()),
        ("dsh", crate::parsers::dsh::discover()),
    ];
    for (source, candidates) in file_sources {
        let total = candidates.len();
        let mut updated = 0usize;
        let mut skipped = 0usize;
        for (i, cand) in candidates.iter().enumerate() {
            match store::session_unchanged(&conn, &cand.id, cand.file_size, cand.file_mtime) {
                Ok(true) => skipped += 1,
                _ => {
                    let parsed = match cand.source {
                        Source::Claude => crate::parsers::claude::parse(cand),
                        Source::Codex => crate::parsers::codex::parse(cand),
                        Source::Dsh => crate::parsers::dsh::parse(cand),
                        _ => unreachable!(),
                    };
                    match parsed {
                        Ok(ps) => {
                            if store::save_parsed(&conn, &ps.meta, &ps.messages, Some(&ps.stats)).is_ok() {
                                updated += 1;
                            }
                        }
                        Err(e) => log::warn!("parse {} failed: {e}", cand.id),
                    }
                }
            }
            if i % 10 == 9 || i + 1 == total {
                let _ = app.emit(
                    "scan-progress",
                    ScanProgress { source: source.into(), done: i + 1, total, updated },
                );
            }
        }
        match source {
            "claude" => summary.claude = updated,
            "codex" => summary.codex = updated,
            "dsh" => summary.dsh = updated,
            _ => {}
        }
        summary.skipped += skipped;
    }

    // ---- zcode:单一 SQLite 库,库级水位判断 + 每会话增量 ----
    if let Some(db) = crate::parsers::zcode::db_path() {
        let _ = app.emit("scan-progress", ScanProgress {
            source: "zcode".into(),
            done: 0,
            total: 0,
            updated: 0,
        });
        match crate::parsers::zcode::open_source(&db) {
            Ok(src) => {
                let stats_map = crate::parsers::zcode::collect_stats(&src);
                let mut updated = 0usize;
                let mut skipped = 0usize;
                match crate::parsers::zcode::list_sessions(&src) {
                    Ok(zcands) => {
                        let total = zcands.len();
                        for (i, zc) in zcands.iter().enumerate() {
                            let cand = crate::parsers::zcode::make_candidate(zc, &db);
                            let unchanged = store::session_unchanged(&conn, &cand.id, 0, cand.file_mtime)
                                .unwrap_or(false);
                            if unchanged {
                                skipped += 1;
                            } else {
                                match crate::parsers::zcode::parse(&src, zc, &cand) {
                                    Ok(mut ps) => {
                                        ps.stats = stats_map.get(&zc.local_id).cloned().unwrap_or_default();
                                        if store::save_parsed(&conn, &ps.meta, &ps.messages, Some(&ps.stats))
                                            .is_ok()
                                        {
                                            updated += 1;
                                        }
                                    }
                                    Err(e) => log::warn!("zcode parse {} failed: {e}", zc.local_id),
                                }
                            }
                            if i % 20 == 19 || i + 1 == total {
                                let _ = app.emit(
                                    "scan-progress",
                                    ScanProgress { source: "zcode".into(), done: i + 1, total, updated },
                                );
                            }
                        }
                    }
                    Err(e) => log::warn!("zcode list_sessions failed: {e}"),
                }
                summary.zcode = updated;
                summary.skipped += skipped;
            }
            Err(e) => log::warn!("zcode open failed: {e}"),
        }
    }

    summary.duration_ms = started.elapsed().as_millis() as u64;
    Ok(summary)
}
