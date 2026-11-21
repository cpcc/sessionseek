//! 文件监听:watch 四家 harness 目录,静默期后触发增量扫描。
//! 扫描本身按 mtime/size 水位跳过未变会话,全量遍历开销很小。

use crate::{scan, AppState};
use notify::{Event, RecommendedWatcher, Watcher};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// 静默窗口:最后一次变更后等待多久才触发扫描。
const QUIET: Duration = Duration::from_secs(3);

pub fn start(app: AppHandle) {
    std::thread::spawn(move || watch_loop(app));
}

fn watch_loop(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher: Option<RecommendedWatcher> =
        notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        })
        .ok();

    let Some(home) = dirs::home_dir() else { return };
    if let Some(w) = watcher.as_mut() {
        // 逐个注册;目录缺失/平台限制导致的失败只记录,不影响其余来源
        let targets = [
            (home.join(".claude/projects"), notify::RecursiveMode::Recursive),
            (home.join(".codex/sessions"), notify::RecursiveMode::Recursive),
            (home.join(".dsh/sessions"), notify::RecursiveMode::Recursive),
            (home.join(".zcode/cli/db"), notify::RecursiveMode::NonRecursive),
        ];
        for (path, mode) in targets {
            if let Err(e) = w.watch(&path, mode) {
                log::warn!("watch {} failed: {e}", path.display());
            } else {
                log::info!("watching {}", path.display());
            }
        }
    } else {
        log::warn!("notify watcher unavailable, incremental scan disabled");
        return;
    }

    loop {
        // 等第一个变更事件
        if rx.recv().is_err() {
            return;
        }
        // 静默窗口:吸收连发事件
        while rx.recv_timeout(QUIET).is_ok() {}

        let state = app.state::<AppState>();
        state.pending_scan.store(true, Ordering::SeqCst);
        // 扫描期间若又有变更,pending 仍为 true → 继续补扫
        while state.pending_scan.swap(false, Ordering::SeqCst) {
            scan::scan_blocking(&app);
        }
    }
}
