//! Tauri 命令。查询各自开只读连接(spawn_blocking),扫描线程持有写连接,互不阻塞。

use crate::model::{Message, SessionMeta, Source};
use crate::scan;
use crate::store;
use crate::AppState;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPage {
    pub items: Vec<SessionMeta>,
    pub total: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    pub meta: SessionMeta,
    pub items: Vec<Message>,
    pub total: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCount {
    pub source: Source,
    pub label: String,
    pub count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCount {
    pub project: String,
    pub count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub sources: Vec<SourceCount>,
    pub projects: Vec<ProjectCount>,
}

async fn with_read_conn<F, T>(state: &State<'_, AppState>, f: F) -> Result<T, String>
where
    F: FnOnce(&rusqlite::Connection) -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    let path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = store::open_read(&path).map_err(|e| e.to_string())?;
        f(&conn)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_sessions(
    state: State<'_, AppState>,
    source: Option<String>,
    project: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<SessionPage, String> {
    with_read_conn(&state, move |conn| {
        store::list_sessions(
            conn,
            source.as_deref(),
            project.as_deref(),
            offset.unwrap_or(0),
            limit.unwrap_or(200),
        )
        .map(|p| SessionPage { items: p.items, total: p.total })
        .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn get_session(
    state: State<'_, AppState>,
    id: String,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<MessagePage, String> {
    with_read_conn(&state, move |conn| {
        let meta = store::get_session_meta(conn, &id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "session not found".to_string())?;
        let page = store::get_messages(conn, &id, offset.unwrap_or(0), limit.unwrap_or(100))
            .map_err(|e| e.to_string())?;
        Ok(MessagePage { meta, items: page.items, total: page.total })
    })
    .await
}

#[tauri::command]
pub async fn get_stats(state: State<'_, AppState>, source: Option<String>) -> Result<Stats, String> {
    with_read_conn(&state, move |conn| {
        let sources = store::source_counts(conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(s, c)| SourceCount { source: s, label: s.label().into(), count: c })
            .collect();
        let projects = store::project_counts(conn, source.as_deref())
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(p, c)| ProjectCount { project: p, count: c })
            .collect();
        Ok(Stats { sources, projects })
    })
    .await
}

#[tauri::command]
pub fn scan_now(app: AppHandle) -> Result<bool, String> {
    let state = app.state::<AppState>();
    if state.scanning.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(false);
    }
    // run_full_scan 内部用 swap 做原子抢占,这里只负责起线程
    let app2 = app.clone();
    std::thread::spawn(move || scan::run_full_scan(&app2));
    Ok(true)
}
