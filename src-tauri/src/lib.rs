mod commands;
mod model;
mod parsers;
mod scan;
mod store;
mod watcher;

use std::sync::atomic::AtomicBool;
use tauri::Manager;

pub struct AppState {
    pub db_path: std::path::PathBuf,
    pub scanning: AtomicBool,
    /// watcher 置位:静默期内有变更,待扫描(扫描期间的新变更也会置位,扫完补扫)
    pub pending_scan: AtomicBool,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app
                .path()
                .app_data_dir()
                .expect("app_data_dir unavailable");
            std::fs::create_dir_all(&dir)?;
            let db_path = dir.join("index.db");
            // 建库建表 + 升级迁移(幂等),后续线程各自连接
            {
                let conn = store::open(&db_path)?;
                store::migrate(&conn)?;
                let _ = conn.close();
            }

            app.manage(AppState {
                db_path,
                scanning: AtomicBool::new(false),
                pending_scan: AtomicBool::new(false),
            });

            // 启动即后台首扫,进度经 scan-progress / scan-finished 事件推送
            let handle = app.handle().clone();
            scan::run_full_scan(&handle);
            // M3:监听 harness 目录变更,静默后自动增量扫描
            watcher::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_sessions,
            commands::get_session,
            commands::get_stats,
            commands::scan_now,
            commands::search_messages,
            commands::get_overview,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
