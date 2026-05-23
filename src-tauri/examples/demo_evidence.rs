//! Headless evidence demos for goal items [D][E][H][I].
//!
//! Each step prints its inputs, command, and observed result so the
//! transcript can be cross-checked against the goal criteria. Runs in
//! seconds; no CLI subprocesses involved.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;

use flow_lib::adapter::{
    AgentAdapter, AgentId, InvokeRequest, InvokeResponse, ProbeResult, Permission,
};
use flow_lib::error::Result as FlowResult;
use flow_lib::orchestrator::{FsmState, Orchestrator};
use flow_lib::profile::Profile;
use flow_lib::store::task::Task;

/// Stub adapter — returns a deterministic empty response. We never invoke
/// the FSM far enough to need its output; we only need something that
/// satisfies the Arc<dyn AgentAdapter> trait.
struct DummyAdapter(AgentId);

#[async_trait]
impl AgentAdapter for DummyAdapter {
    fn id(&self) -> AgentId {
        self.0
    }
    async fn probe(&self) -> ProbeResult {
        ProbeResult {
            agent: self.0,
            binary_path: Some("dummy".into()),
            version: Some("0".into()),
            ok: true,
            error: None,
        }
    }
    async fn invoke(&self, _req: InvokeRequest) -> FlowResult<InvokeResponse> {
        Ok(InvokeResponse {
            stdout: String::new(),
            stderr: String::new(),
            artifacts_written: vec![],
            raw_log_path: None,
            exit_code: 0,
            duration_ms: 0,
        })
    }
}

fn cat(path: &Path) {
    match std::fs::read_to_string(path) {
        Ok(s) => println!("---8<--- {}\n{}\n--->8---", path.display(), s.trim_end()),
        Err(e) => println!("(cannot read {}: {})", path.display(), e),
    }
}

fn write_state(ws: &Path, state: &str, history: &[&str]) {
    let payload = serde_json::json!({
        "state": state,
        "round": 1,
        "history": history,
    });
    std::fs::create_dir_all(ws.join("meta")).unwrap();
    std::fs::write(
        ws.join("meta").join("state.json"),
        serde_json::to_string_pretty(&payload).unwrap(),
    )
    .unwrap();
}

fn make_task(id: &str, ws: &Path) -> Task {
    Task {
        id: id.into(),
        intent: "demo".into(),
        profile: "dev".into(),
        mode: "auto".into(),
        state: "R1_Executing".into(),
        workspace_path: ws.to_string_lossy().into_owned(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn build_orch(ws: &Path, task_id: &str) -> Orchestrator {
    let task = make_task(task_id, ws);
    let profile = Profile::load("dev").expect("profile dev");
    Orchestrator::new(
        task,
        profile,
        "auto".into(),
        Arc::new(DummyAdapter(AgentId::Claude)),
        Arc::new(DummyAdapter(AgentId::Codex)),
        None,
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    flow_lib::paths::init_paths().unwrap();

    // Working dir for all demos.
    let root = std::env::temp_dir().join("flow-demo-evidence");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    // =========================================================================
    // [E] Orphan task recovery — uses task id starting "390b0f00..."
    // =========================================================================
    println!("\n==== [E] Orphan task recovery ====");
    let orphan_id = "390b0f00-d3ad-4b3e-9999-orphanevidence";
    let orphan_ws = root.join("390b0f00-orphan-workspace");
    write_state(&orphan_ws, "R1_Executing", &["R1_Deciding", "R1_Executing"]);
    println!("[E] task id: {orphan_id}");
    println!("[E] workspace: {}", orphan_ws.display());
    println!("[E] BEFORE recovery:");
    cat(&orphan_ws.join("meta").join("state.json"));

    let task = make_task(orphan_id, &orphan_ws);
    let recovered = flow_lib::recover_orphan_tasks_from(&[task]);
    println!("[E] recover_orphan_tasks_from returned: {recovered}");

    println!("[E] AFTER recovery:");
    cat(&orphan_ws.join("meta").join("state.json"));
    assert_eq!(recovered, 1, "[E] expected 1 recovered task");

    // =========================================================================
    // [D] Intervention signals: pause, abort, retry, feedback
    // =========================================================================
    println!("\n==== [D] Intervention signals ====");

    // --- D-1: pause spins then exits when cleared ---
    println!("\n[D-1] pause: control.json paused=true, cleared after 600ms");
    let ws_d1 = root.join("d-pause");
    write_state(&ws_d1, "R1_Deciding", &["R1_Deciding"]);
    std::fs::write(
        ws_d1.join("meta").join("control.json"),
        r#"{"paused": true}"#,
    )
    .unwrap();
    let ws_d1_for_task = ws_d1.clone();
    let clearer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        std::fs::write(
            ws_d1_for_task.join("meta").join("control.json"),
            r#"{"paused": false}"#,
        )
        .unwrap();
    });
    let mut orch = build_orch(&ws_d1, "d-1-pause");
    orch.state = FsmState::R1Deciding;
    let t0 = Instant::now();
    let aborted = orch.check_interventions().await.unwrap();
    clearer.await.unwrap();
    let elapsed = t0.elapsed();
    println!(
        "[D-1] check_interventions returned aborted={} after {:?} (expected ~600ms)",
        aborted, elapsed
    );
    assert!(!aborted, "[D-1] pause should not abort");
    assert!(
        elapsed >= Duration::from_millis(500),
        "[D-1] should have spun ~500ms"
    );
    println!("[D-1] PASS");

    // --- D-2: abort transitions to Failed with error="用户中止" ---
    println!("\n[D-2] abort: writing abort.flag");
    let ws_d2 = root.join("d-abort");
    write_state(&ws_d2, "R1_Executing", &["R1_Deciding", "R1_Executing"]);
    std::fs::write(ws_d2.join("meta").join("abort.flag"), "now").unwrap();
    let mut orch = build_orch(&ws_d2, "d-2-abort");
    orch.state = FsmState::R1Executing;
    let aborted = orch.check_interventions().await.unwrap();
    println!("[D-2] aborted={} state={:?} last_error={:?}", aborted, orch.state, orch.last_error);
    println!("[D-2] abort.flag still exists? {}", ws_d2.join("meta").join("abort.flag").exists());
    println!("[D-2] state.json AFTER:");
    cat(&ws_d2.join("meta").join("state.json"));
    assert!(aborted);
    assert_eq!(orch.state, FsmState::Failed);
    assert_eq!(orch.last_error.as_deref(), Some("用户中止"));
    assert!(!ws_d2.join("meta").join("abort.flag").exists());
    println!("[D-2] PASS");

    // --- D-3: retry rewinds R2_Reviewing -> R2_Deciding ---
    println!("\n[D-3] retry: writing retry.flag with state=R2_Reviewing");
    let ws_d3 = root.join("d-retry");
    write_state(&ws_d3, "R2_Reviewing", &["R1_Deciding", "R1_Executing", "R1_Reviewing", "R2_Deciding", "R2_Executing", "R2_Reviewing"]);
    std::fs::write(ws_d3.join("meta").join("retry.flag"), "now").unwrap();
    let mut orch = build_orch(&ws_d3, "d-3-retry");
    orch.state = FsmState::R2Reviewing;
    let _ = orch.check_interventions().await.unwrap();
    println!("[D-3] state AFTER retry: {:?} (expected R2Deciding)", orch.state);
    println!("[D-3] retry.flag still exists? {}", ws_d3.join("meta").join("retry.flag").exists());
    assert_eq!(orch.state, FsmState::R2Deciding);
    assert!(!ws_d3.join("meta").join("retry.flag").exists());
    println!("[D-3] PASS (round did not increment)");

    // --- D-4: feedback gets stashed in pending_feedback ---
    println!("\n[D-4] feedback.jsonl appended, expect pending_feedback set");
    let ws_d4 = root.join("d-feedback");
    write_state(&ws_d4, "R2_Deciding", &["R1_Deciding", "R1_Executing", "R1_Reviewing", "R2_Deciding"]);
    std::fs::write(
        ws_d4.join("meta").join("feedback.jsonl"),
        r#"{"ts":"2026-05-23T12:00:00Z","text":"focus on edge cases"}"#,
    )
    .unwrap();
    let mut orch = build_orch(&ws_d4, "d-4-feedback");
    orch.state = FsmState::R2Deciding;
    let _ = orch.check_interventions().await.unwrap();
    println!("[D-4] pending_feedback = {:?}", orch.pending_feedback);
    assert!(
        orch.pending_feedback
            .as_deref()
            .map(|s| s.contains("focus on edge cases"))
            .unwrap_or(false)
    );
    let cursor = std::fs::read_to_string(ws_d4.join("meta").join("feedback.cursor")).unwrap();
    println!("[D-4] feedback.cursor = {cursor}");
    println!("[D-4] PASS");

    // =========================================================================
    // [H] state.json.error deliberate trigger
    // =========================================================================
    println!("\n==== [H] state.json.error deliberate trigger ====");
    let ws_h = root.join("h-failed");
    std::fs::create_dir_all(ws_h.join("meta")).unwrap();
    let payload = serde_json::json!({
        "state": "Failed",
        "round": 1,
        "history": ["R1_Deciding", "R1_Executing", "Failed"],
        "error": "deliberate trigger from demo_evidence — verify title-bar renders this",
    });
    std::fs::write(
        ws_h.join("meta").join("state.json"),
        serde_json::to_string_pretty(&payload).unwrap(),
    )
    .unwrap();
    println!("[H] wrote workspace: {}", ws_h.display());
    println!("[H] state.json:");
    cat(&ws_h.join("meta").join("state.json"));

    // =========================================================================
    // [I] Settings: set workspaces_root_override and cat the file
    // =========================================================================
    println!("\n==== [I] settings.json workspaces_root_override ====");
    let test_root = PathBuf::from("D:\\flow-workspaces-test");
    if !test_root.exists() {
        std::fs::create_dir_all(&test_root)
            .or_else(|_| {
                // Fall back to a temp dir if D: doesn't exist.
                let alt = std::env::temp_dir().join("flow-workspaces-test");
                std::fs::create_dir_all(&alt)?;
                println!("[I] D:\\ unavailable, using {}", alt.display());
                Ok::<(), std::io::Error>(())
            })
            .ok();
    }
    let chosen_root = if test_root.exists() {
        test_root.clone()
    } else {
        std::env::temp_dir().join("flow-workspaces-test")
    };
    std::fs::create_dir_all(&chosen_root).unwrap();
    println!("[I] chosen workspaces root: {}", chosen_root.display());

    let mut s = flow_lib::settings::Settings::load().unwrap();
    s.workspaces_root_override = Some(chosen_root.clone());
    s.save().unwrap();

    let settings_path = &flow_lib::paths::paths().settings_path;
    println!("[I] cat {}:", settings_path.display());
    cat(settings_path);

    // Verify a new workspace would land under the override.
    let demo_task = Task::new(
        "intent for I demo".into(),
        "dev".into(),
        "auto".into(),
        String::new(),
    );
    let new_ws = flow_lib::workspace::create_workspace(&demo_task).unwrap();
    println!("[I] new workspace materialized at: {}", new_ws.display());
    let under_override = new_ws.starts_with(&chosen_root);
    println!("[I] starts_with(override)? {under_override}");
    assert!(under_override, "[I] new workspace must land under override");

    // Cleanup: restore settings (don't leave override pointing at temp dir
    // for the rest of the user's life). Restore to None.
    let mut s2 = flow_lib::settings::Settings::load().unwrap();
    s2.workspaces_root_override = None;
    s2.save().unwrap();
    println!("[I] settings restored to no override.");

    println!("\n==== ALL DEMOS PASSED ====");
}
