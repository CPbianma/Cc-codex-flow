pub mod adapter;
pub mod bridge;
pub mod commands;
pub mod error;
pub mod orchestrator;
pub mod paths;
pub mod profile;
pub mod settings;
pub mod store;
pub mod workspace;

use std::path::PathBuf;

use store::task::Task;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "flow_lib=debug,warn".into()),
        )
        .init();

    paths::init_paths().expect("failed to init app paths");
    store::init().expect("failed to init sqlite");

    // [E] Orphan task recovery: this fresh process has no live orchestrators,
    // so any non-terminal task on disk is by definition orphaned.
    match recover_orphan_tasks() {
        Ok(n) => tracing::info!(recovered = n, "orphan task recovery complete"),
        Err(e) => tracing::warn!(error = %e, "orphan task recovery failed"),
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::create_task,
            commands::list_tasks,
            commands::get_task,
            commands::list_workspace_files,
            commands::read_workspace_file,
            commands::probe_agents,
            commands::get_settings,
            commands::set_workspaces_root,
            commands::reset_task,
            commands::start_task,
            commands::get_task_state,
            commands::intervene,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Scan every task in the database and mark non-terminal ones as Failed
/// with the reason "孤儿任务". Returns the number of tasks recovered.
fn recover_orphan_tasks() -> error::Result<usize> {
    let tasks = Task::list_all()?;
    let mut count = 0usize;
    for task in tasks {
        let ws = PathBuf::from(&task.workspace_path);
        if ws.as_os_str().is_empty() || !ws.exists() {
            continue;
        }
        let state_path = ws.join("meta").join("state.json");
        if !state_path.exists() {
            continue;
        }
        let raw = match std::fs::read_to_string(&state_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let val: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let state = val
            .get("state")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        if matches!(state.as_str(), "Done" | "Failed" | "NeedsHuman" | "Pending") {
            continue;
        }
        // Preserve history and append "Failed".
        let mut history: Vec<String> = val
            .get("history")
            .and_then(|h| h.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        history.push("Failed".to_string());
        let new_payload = serde_json::json!({
            "state": "Failed",
            "round": 0,
            "history": history,
            "error": "孤儿任务",
        });
        // Best-effort write.
        if let Err(e) = std::fs::write(
            &state_path,
            serde_json::to_string_pretty(&new_payload).unwrap_or_default(),
        ) {
            tracing::warn!(task_id = %task.id, error = %e, "failed to write state.json");
            continue;
        }
        let _ = val; // silence dead-store on non-trace builds
        count += 1;
    }
    Ok(count)
}
