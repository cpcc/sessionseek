mod commands;
mod model;
mod parsers;
mod scan;
mod store;

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    pub db_path: std::path::PathBuf,
    pub scanning: AtomicBool,
    // 占位:预留共享连接,M1 各命令自开只读连接
    pub _unused: Mutex<()>,
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
            // 建库建表(幂等),后续线程各自连接
            let conn = store::open(&db_path)?;
            let _ = conn.close();

            app.manage(AppState {
                db_path,
                scanning: AtomicBool::new(false),
                _unused: Mutex::new(()),
            });

            // 启动即后台首扫,进度经 scan-progress / scan-finished 事件推送
            let handle = app.handle().clone();
            scan::run_full_scan(&handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_sessions,
            commands::get_session,
            commands::get_stats,
            commands::scan_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
