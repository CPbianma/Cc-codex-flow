use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::adapter::{AgentAdapter, ProbeResult};
use crate::adapter::claude::ClaudeAdapter;
use crate::adapter::codex::CodexAdapter;
use crate::error::{AppError, Result};
use crate::orchestrator::Orchestrator;
use crate::paths::paths;
use crate::profile::Profile;
use crate::settings::Settings;
use crate::store::task::Task;
use crate::workspace;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateTaskInput {
    pub intent: String,
    pub profile: Option<String>,
    pub mode: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CreateTaskOutput {
    pub task: Task,
    pub workspace_path: String,
}

/// Settings view returned to the frontend. Exposes the *effective*
/// `workspaces_root` (override if set, otherwise the default app-data root)
/// as a plain string so the React layer can render it without juggling
/// `PathBuf` ergonomics.
#[derive(Clone, Debug, Serialize)]
pub struct SettingsView {
    pub default_profile: String,
    pub default_mode: String,
    pub workspaces_root: Option<String>,
    pub claude_cli_path: Option<String>,
    pub codex_cli_path: Option<String>,
}

fn settings_view(s: &Settings) -> SettingsView {
    let effective_root = s
        .workspaces_root_override
        .clone()
        .unwrap_or_else(|| paths().workspaces_root.clone());
    SettingsView {
        default_profile: s.default_profile.clone(),
        default_mode: s.default_mode.clone(),
        workspaces_root: Some(effective_root.to_string_lossy().into_owned()),
        claude_cli_path: s
            .claude_cli_path
            .clone()
            .map(|p| p.to_string_lossy().into_owned()),
        codex_cli_path: s
            .codex_cli_path
            .clone()
            .map(|p| p.to_string_lossy().into_owned()),
    }
}

#[tauri::command]
pub async fn create_task(input: CreateTaskInput) -> Result<CreateTaskOutput> {
    let settings = Settings::load()?;
    let profile = input.profile.unwrap_or(settings.default_profile);
    let mode = input.mode.unwrap_or(settings.default_mode);

    // We allocate the id + create db row first, then materialize the workspace
    // so paths can reference the canonical id.
    let mut task = Task::new(input.intent, profile, mode, String::new());
    let ws = workspace::create_workspace(&task)?;
    task.workspace_path = ws.to_string_lossy().into_owned();
    task.insert()?;

    Ok(CreateTaskOutput {
        workspace_path: task.workspace_path.clone(),
        task,
    })
}

#[tauri::command]
pub async fn list_tasks(limit: Option<i64>) -> Result<Vec<Task>> {
    Task::list_recent(limit.unwrap_or(50))
}

#[tauri::command]
pub async fn get_task(id: String) -> Result<Task> {
    Task::get(&id)
}

#[tauri::command]
pub async fn list_workspace_files(id: String) -> Result<Vec<String>> {
    let task = Task::get(&id)?;
    workspace::list_files(std::path::Path::new(&task.workspace_path))
}

#[tauri::command]
pub async fn read_workspace_file(id: String, relative_path: String) -> Result<String> {
    let task = Task::get(&id)?;
    let ws = std::path::Path::new(&task.workspace_path);
    // Defence-in-depth against path traversal: resolve the requested path
    // and verify it stays inside the workspace. Reject absolute inputs and
    // any `..` that climbs above the workspace root.
    let joined = ws.join(&relative_path);
    let ws_can = std::fs::canonicalize(ws).unwrap_or_else(|_| ws.to_path_buf());
    let target_can = std::fs::canonicalize(&joined).unwrap_or(joined.clone());
    if !target_can.starts_with(&ws_can) {
        return Err(AppError::Other(format!(
            "拒绝越界访问: {}",
            relative_path
        )));
    }
    Ok(std::fs::read_to_string(&target_can)?)
}

#[tauri::command]
pub async fn probe_agents() -> Result<Vec<ProbeResult>> {
    let s = Settings::load()?;
    let claude = ClaudeAdapter::new(s.claude_cli_path.map(|p| p.to_string_lossy().into_owned()));
    let codex = CodexAdapter::new(s.codex_cli_path.map(|p| p.to_string_lossy().into_owned()));
    Ok(vec![claude.probe().await, codex.probe().await])
}

#[tauri::command]
pub async fn get_settings() -> Result<SettingsView> {
    Ok(settings_view(&Settings::load()?))
}

/// Update the workspaces root override. The path is resolved (canonicalize
/// optional — we accept the user's chosen path verbatim, only ensuring it
/// exists and is a directory). Future tasks will be created underneath.
#[tauri::command]
pub async fn set_workspaces_root(path: String) -> Result<SettingsView> {
    let pb = std::path::PathBuf::from(&path);
    if !pb.exists() {
        return Err(AppError::Other(format!("路径不存在: {path}")));
    }
    if !pb.is_dir() {
        return Err(AppError::Other(format!("不是目录: {path}")));
    }
    let mut s = Settings::load()?;
    s.workspaces_root_override = Some(pb);
    s.save()?;
    Ok(settings_view(&s))
}

/// Reset a task back to `Pending` so the user can start it over. We truncate
/// `meta/state.json` and `meta/turns.jsonl` but deliberately keep
/// `decisions/`, `execution/`, `discussion/`, `artifacts/` and `intent.md`
/// — the user might still want them as reference.
///
/// Race-safety: a running orchestrator periodically writes `state.json` from
/// its in-memory FSM, so a naive reset can be silently clobbered. We first
/// drop an `abort.flag` (the orchestrator's intervention poll picks this up
/// within ~half a turn and transitions itself to `Failed`), then refuse the
/// reset unless the persisted `state.json` is in a terminal state. The
/// frontend retries / surfaces the error.
#[tauri::command]
pub async fn reset_task(id: String) -> Result<()> {
    let task = Task::get(&id)?;
    let ws = std::path::Path::new(&task.workspace_path);
    let meta = ws.join("meta");
    if !meta.exists() {
        std::fs::create_dir_all(&meta)?;
    }

    // Inspect current persisted state; bail out (after raising the abort
    // flag) if the FSM is mid-run so we don't race the orchestrator.
    let state_path = meta.join("state.json");
    if state_path.exists() {
        let raw = std::fs::read_to_string(&state_path)?;
        let cur: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
        let state_str = cur
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("Pending")
            .to_string();
        let is_terminal = matches!(
            state_str.as_str(),
            "Pending" | "Done" | "Failed" | "NeedsHuman"
        );
        if !is_terminal {
            // Signal abort so the orchestrator unwinds itself, then ask the
            // caller to retry once the FSM has settled. Also clear any
            // paused flag — if the orchestrator was blocked in the pause
            // spin-loop, it would never poll abort.flag.
            std::fs::write(meta.join("abort.flag"), chrono::Utc::now().to_rfc3339())?;
            std::fs::write(
                meta.join("control.json"),
                serde_json::to_string_pretty(&serde_json::json!({ "paused": false }))?,
            )?;
            return Err(AppError::Other(format!(
                "任务正在运行中（{state_str}），已发送中止信号，请稍候再试"
            )));
        }
    }

    std::fs::write(
        &state_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "state": "Pending",
            "round": 0,
            "history": [],
        }))?,
    )?;
    // Truncate (don't delete) the turn log.
    std::fs::write(meta.join("turns.jsonl"), "")?;
    // Clear stale control / feedback cursors so the next run starts clean.
    // We also truncate feedback.jsonl itself so an aborted run's guidance
    // doesn't leak into the next R1 Decider's user_message.
    for stale in ["abort.flag", "retry.flag", "feedback.cursor", "control.json"] {
        let p = meta.join(stale);
        if p.exists() {
            let _ = std::fs::remove_file(&p);
        }
    }
    let fb = meta.join("feedback.jsonl");
    if fb.exists() {
        std::fs::write(&fb, "")?;
    }
    Ok(())
}

/// Spawn the FSM for an existing task on the tokio runtime and return
/// immediately. Progress is reported via the `task.state_changed` event.
#[tauri::command]
pub async fn start_task(id: String, app: tauri::AppHandle) -> Result<()> {
    let task = Task::get(&id)?;
    let profile = Profile::load(&task.profile)?;
    let settings = Settings::load()?;
    let mode = task.mode.clone();

    let claude: Arc<dyn AgentAdapter> = Arc::new(ClaudeAdapter::new(
        settings.claude_cli_path.map(|p| p.to_string_lossy().into_owned()),
    ));
    let codex: Arc<dyn AgentAdapter> = Arc::new(CodexAdapter::new(
        settings.codex_cli_path.map(|p| p.to_string_lossy().into_owned()),
    ));

    let mut orch = Orchestrator::new(task, profile, mode, claude, codex, Some(app));

    // Detached background run — failures are logged but don't propagate to
    // the IPC caller (they show up in state.json + the task.state_changed
    // event stream instead).
    tokio::spawn(async move {
        if let Err(e) = orch.run().await {
            tracing::error!(error = %e, "orchestrator run failed");
        }
    });
    Ok(())
}

/// Read the FSM `state.json` for a task and return it verbatim.
#[tauri::command]
pub async fn get_task_state(id: String) -> Result<String> {
    let task = Task::get(&id)?;
    let path = std::path::Path::new(&task.workspace_path)
        .join("meta")
        .join("state.json");
    if !path.exists() {
        return Err(AppError::NotFound(format!("state.json for {id}")));
    }
    Ok(std::fs::read_to_string(&path)?)
}

/// Force-delete a task: clear the DB row and archive the on-disk workspace.
///
/// If the FSM is mid-run we don't wait for it to settle. We:
///   1. raise `abort.flag` + clear `paused` so the orchestrator unwinds at its
///      next poll,
///   2. on Windows, taskkill any subprocess whose CommandLine references the
///      workspace path (claude/codex CLI children) so the rename doesn't hit a
///      sharing violation,
///   3. delete the DB row first (the task disappears from the list immediately
///      — visual progress for the user),
///   4. best-effort archive the workspace into `<root>/_archive/<ts>-<slug>/`,
///      stripping `meta/`, `discussion/`, `CLAUDE.md`, `AGENTS.md`,
///      `mcp.shared.json`. One retry with another subprocess sweep if the
///      first rename fails. Archive failure is logged, not propagated — the
///      task is already gone from the UI.
#[tauri::command]
pub async fn delete_task(id: String) -> Result<()> {
    let task = Task::get(&id)?;
    let ws = std::path::PathBuf::from(&task.workspace_path);

    if ws.exists() {
        let meta = ws.join("meta");
        if meta.exists() {
            let _ = std::fs::write(
                meta.join("abort.flag"),
                chrono::Utc::now().to_rfc3339(),
            );
            let _ = std::fs::write(
                meta.join("control.json"),
                serde_json::to_string(&serde_json::json!({ "paused": false }))?,
            );
        }
        kill_subprocesses_using_workspace(&task.workspace_path);
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    }

    Task::delete(&id)?;

    if ws.exists() {
        if let Err(e) = workspace::archive_workspace(&ws, &task.intent) {
            kill_subprocesses_using_workspace(&task.workspace_path);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if let Err(e2) = workspace::archive_workspace(&ws, &task.intent) {
                tracing::warn!(
                    task_id = %id,
                    first = %e,
                    second = %e2,
                    "archive failed twice; workspace folder retained on disk"
                );
            }
        }
    }
    Ok(())
}

/// Windows-only: kill any process whose CommandLine references the workspace
/// path. Used by [`delete_task`] to free the directory before rename.
///
/// Best-effort: a single PowerShell invocation, no error propagation.
#[cfg(windows)]
fn kill_subprocesses_using_workspace(ws_path: &str) {
    let escaped = ws_path.replace('\'', "''");
    let script = format!(
        "Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*{}*' }} | ForEach-Object {{ try {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction Stop }} catch {{}} }}",
        escaped
    );
    let _ = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();
}

#[cfg(not(windows))]
fn kill_subprocesses_using_workspace(_ws_path: &str) {}

/// Human-in-the-loop intervention. Records the action against the
/// workspace; the FSM picks it up on its next iteration (Phase-3 wiring).
///
/// Recognised action tags:
///   * `pause`  / `resume` — toggles `meta/control.json { paused: bool }`
///   * `abort`              — writes `meta/abort.flag`
///   * `retry`              — writes `meta/retry.flag`
///   * `feedback:<text>`    — appended to `meta/feedback.jsonl`
/// Every action is also appended verbatim to `meta/interventions.jsonl`.
#[tauri::command]
pub async fn intervene(id: String, action: String) -> Result<()> {
    let task = Task::get(&id)?;
    let meta = std::path::Path::new(&task.workspace_path).join("meta");
    std::fs::create_dir_all(&meta)?;

    // Always append to the canonical log.
    {
        let path = meta.join("interventions.jsonl");
        let line = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "action": action,
        });
        let mut content = serde_json::to_string(&line)?;
        content.push('\n');
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        f.write_all(content.as_bytes())?;
    }

    // Per-action side-effects.
    match action.as_str() {
        "pause" => {
            std::fs::write(
                meta.join("control.json"),
                serde_json::to_string_pretty(&serde_json::json!({ "paused": true }))?,
            )?;
        }
        "resume" => {
            std::fs::write(
                meta.join("control.json"),
                serde_json::to_string_pretty(&serde_json::json!({ "paused": false }))?,
            )?;
        }
        "abort" => {
            std::fs::write(meta.join("abort.flag"), chrono::Utc::now().to_rfc3339())?;
        }
        "retry" => {
            std::fs::write(meta.join("retry.flag"), chrono::Utc::now().to_rfc3339())?;
        }
        s if s.starts_with("feedback:") => {
            let body = &s["feedback:".len()..];
            let line = serde_json::json!({
                "ts": chrono::Utc::now().to_rfc3339(),
                "text": body,
            });
            let mut content = serde_json::to_string(&line)?;
            content.push('\n');
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(meta.join("feedback.jsonl"))?;
            f.write_all(content.as_bytes())?;
        }
        _ => {}
    }
    Ok(())
}
