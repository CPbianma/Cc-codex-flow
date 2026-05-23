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
    Ok(recover_orphan_tasks_from(&tasks))
}

/// Pure file-touching half of [`recover_orphan_tasks`]: given an explicit list
/// of tasks, mark each non-terminal task's `meta/state.json` as
/// `Failed`/"孤儿任务" and return the count of mutated files.
///
/// Factored out so the recovery loop can be exercised by unit tests against a
/// hand-built workspace without needing the sqlite store.
pub fn recover_orphan_tasks_from(tasks: &[Task]) -> usize {
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
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// Build a throwaway directory under the OS temp dir. Caller is
    /// responsible for cleaning it up with `std::fs::remove_dir_all`.
    fn fresh_tempdir(tag: &str) -> PathBuf {
        let unique = uuid::Uuid::new_v4().to_string();
        let dir = std::env::temp_dir().join(format!("flow-test-{tag}-{unique}"));
        std::fs::create_dir_all(&dir).expect("create tempdir");
        dir
    }

    fn make_task(workspace: &std::path::Path) -> Task {
        Task {
            id: uuid::Uuid::new_v4().to_string(),
            intent: "test".into(),
            profile: "dev".into(),
            mode: "auto".into(),
            state: "R1_Executing".into(),
            workspace_path: workspace.to_string_lossy().into_owned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn recover_orphan_marks_nonterminal_state_failed() {
        let ws = fresh_tempdir("orphan-nonterm");
        std::fs::create_dir_all(ws.join("meta")).unwrap();
        std::fs::write(
            ws.join("meta").join("state.json"),
            serde_json::json!({
                "state": "R1_Executing",
                "round": 1,
                "history": ["Pending", "R1_Deciding", "R1_Executing"],
            })
            .to_string(),
        )
        .unwrap();
        let task = make_task(&ws);

        let n = recover_orphan_tasks_from(&[task]);
        assert_eq!(n, 1);

        let raw = std::fs::read_to_string(ws.join("meta").join("state.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["state"], "Failed");
        assert_eq!(v["error"], "孤儿任务");
        // History was preserved and appended.
        let hist: Vec<String> = v["history"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        assert_eq!(hist.last().unwrap(), "Failed");
        assert!(hist.contains(&"R1_Executing".to_string()));

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn recover_orphan_skips_terminal_state() {
        let ws = fresh_tempdir("orphan-term");
        std::fs::create_dir_all(ws.join("meta")).unwrap();
        let original = serde_json::json!({
            "state": "Done",
            "round": 1,
            "history": ["Pending", "Done"],
        });
        std::fs::write(
            ws.join("meta").join("state.json"),
            original.to_string(),
        )
        .unwrap();
        let task = make_task(&ws);

        let n = recover_orphan_tasks_from(&[task]);
        assert_eq!(n, 0);
        // Unchanged.
        let raw = std::fs::read_to_string(ws.join("meta").join("state.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["state"], "Done");
        assert!(v.get("error").map(|e| e.is_null()).unwrap_or(true));

        let _ = std::fs::remove_dir_all(&ws);
    }
}
