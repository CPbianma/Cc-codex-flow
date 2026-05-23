//! [D] Live intervention-signal demo.
//!
//! Exercises Orchestrator::check_interventions() by writing the four
//! intervention files (control.json / abort.flag / retry.flag /
//! feedback.jsonl) into a fresh workspace and asserting the resulting
//! orchestrator state and side effects.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use flow_lib::adapter::{
    AgentAdapter, AgentId, InvokeRequest, InvokeResponse, Permission, ProbeResult,
};
use flow_lib::error::Result;
use flow_lib::orchestrator::{fsm::FsmState, Orchestrator};
use flow_lib::paths::{init_paths, paths};
use flow_lib::profile::{Profile, RoleSpec, SwapSpec};
use flow_lib::store::task::Task;

/// Canned adapter — never spawns, returns a benign InvokeResponse.
struct DummyAdapter {
    id: AgentId,
}

#[async_trait]
impl AgentAdapter for DummyAdapter {
    fn id(&self) -> AgentId {
        self.id
    }
    async fn probe(&self) -> ProbeResult {
        ProbeResult {
            agent: self.id,
            binary_path: None,
            version: Some("dummy".into()),
            ok: true,
            error: None,
        }
    }
    async fn invoke(&self, _req: InvokeRequest) -> Result<InvokeResponse> {
        Ok(InvokeResponse {
            stdout: String::new(),
            stderr: String::new(),
            artifacts_written: Vec::new(),
            raw_log_path: None,
            exit_code: 0,
            duration_ms: 0,
        })
    }
}

fn dummy_profile() -> Profile {
    let spec = RoleSpec {
        agent: AgentId::Claude,
        template_path: "templates/dev-decider.md".into(),
        artifacts: vec![],
    };
    Profile {
        name: "dev".into(),
        description: String::new(),
        default_permission: Permission::ReadOnly,
        decider: spec.clone(),
        executor: RoleSpec {
            agent: AgentId::Codex,
            template_path: "templates/dev-executor.md".into(),
            artifacts: vec![],
        },
        reviewer: spec,
        swap: SwapSpec::default(),
    }
}

fn fresh_workspace(task_id: &str) -> Result<PathBuf> {
    let ws = paths().workspaces_root.join(task_id);
    // Clean prior runs.
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join("meta"))?;
    std::fs::create_dir_all(ws.join("decisions"))?;
    std::fs::write(
        ws.join("meta").join("state.json"),
        r#"{"state":"R1_Deciding","round":1,"history":[]}"#,
    )?;
    Ok(ws)
}

fn make_orch(ws: &std::path::Path) -> Orchestrator {
    let task = Task {
        id: ws.file_name().unwrap().to_string_lossy().into_owned(),
        intent: "intervene demo".into(),
        profile: "dev".into(),
        mode: "auto".into(),
        state: "Pending".into(),
        workspace_path: ws.to_string_lossy().into_owned(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    Orchestrator::new(
        task,
        dummy_profile(),
        "auto".into(),
        Arc::new(DummyAdapter { id: AgentId::Claude }),
        Arc::new(DummyAdapter { id: AgentId::Codex }),
        None,
    )
}

fn read_state(ws: &std::path::Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(ws.join("meta").join("state.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    init_paths()?;

    let task_id = "d111111d-1111-1111-1111-111111111111";
    let ws = fresh_workspace(task_id)?;
    println!("[D] workspace: {}", ws.display());

    // ─── step 1: pause ────────────────────────────────────────────────────
    {
        let mut orch = make_orch(&ws);
        orch.state = FsmState::R1Deciding;
        // Write paused=true then flip from a background task so we don't hang.
        std::fs::write(
            ws.join("meta").join("control.json"),
            serde_json::json!({"paused": true}).to_string(),
        )?;
        let ctrl = ws.join("meta").join("control.json");
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = std::fs::write(&ctrl, serde_json::json!({"paused": false}).to_string());
        });
        let aborted = orch.check_interventions().await?;
        if !aborted && orch.state == FsmState::R1Deciding {
            println!("[D-step-1] PASS pause held loop, state unchanged (R1Deciding)");
        } else {
            println!("[D-step-1] FAIL aborted={aborted} state={:?}", orch.state);
        }
        let _ = std::fs::remove_file(ws.join("meta").join("control.json"));
    }

    // ─── step 2: abort ────────────────────────────────────────────────────
    {
        // Reset state.json so persist_state can overwrite cleanly.
        std::fs::write(
            ws.join("meta").join("state.json"),
            r#"{"state":"R1_Executing","round":1,"history":[]}"#,
        )?;
        let mut orch = make_orch(&ws);
        orch.state = FsmState::R1Executing;
        std::fs::write(ws.join("meta").join("abort.flag"), "x")?;
        let aborted = orch.check_interventions().await?;
        let flag_gone = !ws.join("meta").join("abort.flag").exists();
        if aborted
            && orch.state == FsmState::Failed
            && orch.last_error.as_deref() == Some("用户中止")
            && flag_gone
        {
            println!(
                "[D-step-2] PASS aborted=true state=Failed last_error=用户中止 flag-removed=true"
            );
        } else {
            println!(
                "[D-step-2] FAIL aborted={aborted} state={:?} last_error={:?} flag-removed={flag_gone}",
                orch.state, orch.last_error
            );
        }
        let v = read_state(&ws);
        println!("[D-step-2] state.json AFTER: {}", serde_json::to_string(&v)?);
    }

    // ─── step 3: retry rewinds Reviewing -> Deciding ─────────────────────
    {
        std::fs::write(
            ws.join("meta").join("state.json"),
            r#"{"state":"R2_Reviewing","round":2,"history":[]}"#,
        )?;
        let mut orch = make_orch(&ws);
        orch.state = FsmState::R2Reviewing;
        std::fs::write(ws.join("meta").join("retry.flag"), "x")?;
        let aborted = orch.check_interventions().await?;
        let flag_gone = !ws.join("meta").join("retry.flag").exists();
        if !aborted && orch.state == FsmState::R2Deciding && flag_gone {
            println!("[D-step-3] PASS state rewound R2Reviewing -> R2Deciding, flag removed");
        } else {
            println!(
                "[D-step-3] FAIL aborted={aborted} state={:?} flag-removed={flag_gone}",
                orch.state
            );
        }
    }

    // ─── step 4: feedback file populates pending_feedback ────────────────
    {
        std::fs::write(
            ws.join("meta").join("state.json"),
            r#"{"state":"R2_Deciding","round":2,"history":[]}"#,
        )?;
        // Clear cursor from prior runs.
        let _ = std::fs::remove_file(ws.join("meta").join("feedback.cursor"));
        let mut orch = make_orch(&ws);
        orch.state = FsmState::R2Deciding;
        std::fs::write(
            ws.join("meta").join("feedback.jsonl"),
            r#"{"ts":"2026-05-23T00:00:00Z","text":"please focus on edge cases"}
"#,
        )?;
        let aborted = orch.check_interventions().await?;
        let pf = orch.pending_feedback.clone().unwrap_or_default();
        if !aborted && pf.contains("please focus on edge cases") {
            println!("[D-step-4] PASS pending_feedback contains expected text: {pf:?}");
        } else {
            println!(
                "[D-step-4] FAIL aborted={aborted} pending_feedback={:?}",
                orch.pending_feedback
            );
        }
    }

    // ─── step 5: idempotent — no signals = no-op ─────────────────────────
    {
        // Reset state and clear all signal files.
        std::fs::write(
            ws.join("meta").join("state.json"),
            r#"{"state":"R1_Executing","round":1,"history":[]}"#,
        )?;
        let _ = std::fs::remove_file(ws.join("meta").join("abort.flag"));
        let _ = std::fs::remove_file(ws.join("meta").join("retry.flag"));
        let _ = std::fs::remove_file(ws.join("meta").join("control.json"));
        let _ = std::fs::remove_file(ws.join("meta").join("feedback.jsonl"));
        let _ = std::fs::remove_file(ws.join("meta").join("feedback.cursor"));
        let mut orch = make_orch(&ws);
        orch.state = FsmState::R1Executing;
        let aborted = orch.check_interventions().await?;
        if !aborted && orch.state == FsmState::R1Executing {
            println!("[D-step-5] PASS no-signal call is a no-op, state unchanged");
        } else {
            println!("[D-step-5] FAIL aborted={aborted} state={:?}", orch.state);
        }
    }

    Ok(())
}
