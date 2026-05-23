//! Phase-3 FSM driver.
//!
//! Walks rounds R1 → R2 → R3 (with role swap) → R4 (human gate),
//! parses real reviewer verdicts from `decisions/{n:03}-review.md`,
//! and honours intervention signals written to `meta/` by the
//! `intervene` Tauri command (pause / abort / retry / feedback).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::adapter::{
    AgentAdapter, AgentId, InvokeRequest, InvokeResponse, Permission, Role,
};
use crate::bridge::role::{render_role, write_contract};
use crate::error::{AppError, Result};
use crate::profile::{Profile, RoleSpec};
use crate::store::task::Task;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FsmState {
    Pending,
    R1Deciding,
    R1Executing,
    R1Reviewing,
    R2Deciding,
    R2Executing,
    R2Reviewing,
    R3Deciding,
    R3Executing,
    R3Reviewing,
    NeedsHuman,
    Done,
    Failed,
}

impl FsmState {
    pub fn as_str(&self) -> &'static str {
        match self {
            FsmState::Pending => "Pending",
            FsmState::R1Deciding => "R1_Deciding",
            FsmState::R1Executing => "R1_Executing",
            FsmState::R1Reviewing => "R1_Reviewing",
            FsmState::R2Deciding => "R2_Deciding",
            FsmState::R2Executing => "R2_Executing",
            FsmState::R2Reviewing => "R2_Reviewing",
            FsmState::R3Deciding => "R3_Deciding",
            FsmState::R3Executing => "R3_Executing",
            FsmState::R3Reviewing => "R3_Reviewing",
            FsmState::NeedsHuman => "NeedsHuman",
            FsmState::Done => "Done",
            FsmState::Failed => "Failed",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatePayload {
    pub state: String,
    pub round: u32,
    pub history: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct Orchestrator {
    pub task: Task,
    pub profile: Profile,
    pub mode: String,
    pub claude: Arc<dyn AgentAdapter>,
    pub codex: Arc<dyn AgentAdapter>,
    pub app_handle: Option<tauri::AppHandle>,
    pub state: FsmState,
    pub history: Vec<String>,
    pub last_error: Option<String>,
    pub pending_feedback: Option<String>,
}

impl Orchestrator {
    pub fn new(
        task: Task,
        profile: Profile,
        mode: String,
        claude: Arc<dyn AgentAdapter>,
        codex: Arc<dyn AgentAdapter>,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        Self {
            task,
            profile,
            mode,
            claude,
            codex,
            app_handle,
            state: FsmState::Pending,
            history: Vec::new(),
            last_error: None,
            pending_feedback: None,
        }
    }

    /// Run the FSM. In `auto` mode walk straight through to a terminal
    /// state; in `checkpoint` mode return after each round so the caller
    /// can resume via [`step`].
    pub async fn run(&mut self) -> Result<()> {
        if let Err(e) = self.run_inner().await {
            tracing::error!(error = %e, "FSM hit error, transitioning to Failed");
            self.last_error = Some(e.to_string());
            // Best-effort transition to Failed; never propagate the original
            // error past this point so the spawned tokio task in start_task
            // can finish cleanly. The error is visible in state.json.
            let _ = self.transition(FsmState::Failed).await;
            return Ok(());
        }
        Ok(())
    }

    async fn run_inner(&mut self) -> Result<()> {
        self.transition(FsmState::R1Deciding).await?;
        loop {
            // [D] Poll intervention signals before every step.
            if self.check_interventions().await? {
                // Aborted — terminal state already set, exit loop.
                return Ok(());
            }
            let prev = self.state;
            self.step().await?;
            if matches!(
                self.state,
                FsmState::Done | FsmState::Failed | FsmState::NeedsHuman
            ) {
                break;
            }
            // checkpoint: pause after every round-ending transition.
            if self.mode == "checkpoint" && round_boundary(prev, self.state) {
                break;
            }
        }
        Ok(())
    }

    /// Poll `<workspace>/meta/` for `control.json` (pause), `abort.flag`,
    /// `retry.flag`, and `feedback.jsonl`. Returns Ok(true) if the FSM was
    /// aborted (caller should exit the loop).
    pub async fn check_interventions(&mut self) -> Result<bool> {
        let meta = PathBuf::from(&self.task.workspace_path).join("meta");

        // Pause: spin until cleared.
        loop {
            let control = meta.join("control.json");
            let paused = if control.exists() {
                std::fs::read_to_string(&control)
                    .ok()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    .and_then(|v| v.get("paused").and_then(|p| p.as_bool()))
                    .unwrap_or(false)
            } else {
                false
            };
            if !paused {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        // Abort.
        let abort = meta.join("abort.flag");
        if abort.exists() {
            let _ = std::fs::remove_file(&abort);
            self.last_error = Some("用户中止".into());
            self.transition(FsmState::Failed).await?;
            return Ok(true);
        }

        // Retry: rewind Executing/Reviewing back to Deciding of the same round.
        let retry = meta.join("retry.flag");
        if retry.exists() {
            let _ = std::fs::remove_file(&retry);
            let new_state = match self.state {
                FsmState::R1Executing | FsmState::R1Reviewing => Some(FsmState::R1Deciding),
                FsmState::R2Executing | FsmState::R2Reviewing => Some(FsmState::R2Deciding),
                FsmState::R3Executing | FsmState::R3Reviewing => Some(FsmState::R3Deciding),
                FsmState::R1Deciding | FsmState::R2Deciding | FsmState::R3Deciding => None,
                _ => None,
            };
            if let Some(s) = new_state {
                self.transition(s).await?;
            }
        }

        // Feedback: pull unread entries via byte-offset cursor.
        let feedback = meta.join("feedback.jsonl");
        if feedback.exists() {
            let cursor_path = meta.join("feedback.cursor");
            let cursor: u64 = std::fs::read_to_string(&cursor_path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            let full = std::fs::read(&feedback)?;
            let total_len = full.len() as u64;
            if total_len > cursor {
                let tail = &full[cursor as usize..];
                // Only consume up to the last complete line; any partial trailing
                // line (writer mid-flush) stays for the next poll. This avoids the
                // data-loss bug where a partial line that fails parse silently
                // bumps the cursor past those bytes.
                let last_newline = tail.iter().rposition(|&b| b == b'\n');
                let consumable = match last_newline {
                    Some(i) => &tail[..=i],
                    None => &[][..],
                };
                let consumed_len = consumable.len() as u64;
                let mut parts: Vec<String> = Vec::new();
                for line in consumable.split(|b| *b == b'\n') {
                    if line.is_empty() {
                        continue;
                    }
                    let s = match std::str::from_utf8(line) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
                        if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
                            parts.push(t.to_string());
                        }
                    }
                }
                if !parts.is_empty() {
                    let combined = parts.join("\n---\n");
                    // Append to any pre-existing pending feedback so nothing is lost.
                    self.pending_feedback = Some(match self.pending_feedback.take() {
                        Some(prev) => format!("{prev}\n---\n{combined}"),
                        None => combined,
                    });
                }
                if consumed_len > 0 {
                    std::fs::write(&cursor_path, (cursor + consumed_len).to_string())?;
                }
            }
        }

        Ok(false)
    }

    /// Advance the FSM by exactly one state.
    pub async fn step(&mut self) -> Result<()> {
        let next = match self.state {
            FsmState::Pending => FsmState::R1Deciding,
            FsmState::R1Deciding => {
                self.run_role(1, Role::Decider).await?;
                FsmState::R1Executing
            }
            FsmState::R1Executing => {
                self.run_role(1, Role::Executor).await?;
                FsmState::R1Reviewing
            }
            FsmState::R1Reviewing => {
                let pass = self.run_role(1, Role::Reviewer).await?;
                if pass { FsmState::Done } else { FsmState::R2Deciding }
            }
            FsmState::R2Deciding => {
                self.run_role(2, Role::Decider).await?;
                FsmState::R2Executing
            }
            FsmState::R2Executing => {
                self.run_role(2, Role::Executor).await?;
                FsmState::R2Reviewing
            }
            FsmState::R2Reviewing => {
                let pass = self.run_role(2, Role::Reviewer).await?;
                if pass { FsmState::Done } else { FsmState::R3Deciding }
            }
            FsmState::R3Deciding => {
                self.run_role(3, Role::Decider).await?;
                FsmState::R3Executing
            }
            FsmState::R3Executing => {
                self.run_role(3, Role::Executor).await?;
                FsmState::R3Reviewing
            }
            FsmState::R3Reviewing => {
                let pass = self.run_role(3, Role::Reviewer).await?;
                if pass { FsmState::Done } else { FsmState::NeedsHuman }
            }
            FsmState::NeedsHuman | FsmState::Done | FsmState::Failed => self.state,
        };
        self.transition(next).await?;
        Ok(())
    }

    async fn run_role(&mut self, round: u32, role: Role) -> Result<bool> {
        // Clear any prior reviewer-FAIL reason; it only belongs to its own round.
        if !matches!(role, Role::Reviewer) {
            self.last_error = None;
        }

        let spec = self.spec_for(round, role);
        let agent_id = spec.agent;
        let adapter: Arc<dyn AgentAdapter> = match agent_id {
            AgentId::Claude => self.claude.clone(),
            AgentId::Codex => self.codex.clone(),
        };

        // Render the role contract template.
        let template = self.load_template(&spec.template_path)?;
        let mut vars: HashMap<String, String> = HashMap::new();
        vars.insert("round".into(), round.to_string());
        vars.insert("n".into(), format!("{:03}", round));
        vars.insert("task_id".into(), self.task.id.clone());
        vars.insert("role".into(), format!("{:?}", role));
        let rendered = render_role(&template, &vars);

        // Persist as CLAUDE.md / AGENTS.md for the relevant agent.
        let workspace = PathBuf::from(&self.task.workspace_path);
        write_contract(&workspace, agent_id, &rendered)?;

        let turn_id = format!("r{round}-{}", role_short(role));
        let mcp = workspace.join("mcp.shared.json");

        // Build the user message. If we are entering the Decider and have
        // pending user feedback, prepend it and clear.
        let mut user_message = build_user_message(role, round, &self.task.intent);
        if matches!(role, Role::Decider) {
            if let Some(fb) = self.pending_feedback.take() {
                user_message = format!("用户上一轮的反馈：\n{fb}\n\n{user_message}");
            }
        }

        let req = InvokeRequest {
            workspace: workspace.clone(),
            role,
            system_prompt: rendered,
            user_message,
            permission: self.profile.default_permission,
            mcp_config_path: if mcp.exists() { Some(mcp) } else { None },
            turn_id: turn_id.clone(),
        };

        let started = SystemTime::now();
        let resp = adapter.invoke(req).await?;
        self.append_turn(&turn_id, round, role, agent_id, started, &resp)?;

        // [C] Reviewer verdict: parse decisions/{n:03}-review.md.
        if matches!(role, Role::Reviewer) {
            let review_path = workspace
                .join("decisions")
                .join(format!("{:03}-review.md", round));
            let (pass, reason) = parse_review_verdict(&review_path);
            if pass {
                self.last_error = None;
                return Ok(true);
            } else {
                self.last_error = Some(reason);
                return Ok(false);
            }
        }
        Ok(false)
    }

    fn spec_for(&self, round: u32, role: Role) -> RoleSpec {
        if round == 3 {
            // Role swap — pick the swap target if specified, otherwise fall
            // back to the standard spec but flip the agent.
            let standard = match role {
                Role::Decider => &self.profile.decider,
                Role::Executor => &self.profile.executor,
                Role::Reviewer => &self.profile.reviewer,
            };
            let swapped_agent = match role {
                Role::Decider => self.profile.swap.decider,
                Role::Executor => self.profile.swap.executor,
                Role::Reviewer => self.profile.swap.reviewer,
            };
            let mut s = standard.clone();
            if let Some(a) = swapped_agent {
                s.agent = a;
            }
            s
        } else {
            match role {
                Role::Decider => self.profile.decider.clone(),
                Role::Executor => self.profile.executor.clone(),
                Role::Reviewer => self.profile.reviewer.clone(),
            }
        }
    }

    fn load_template(&self, rel: &str) -> Result<String> {
        // Resolve the template relative to the built-in profiles dir first;
        // user-supplied absolute paths win if present.
        let candidate = PathBuf::from(rel);
        let path = if candidate.is_absolute() {
            candidate
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("profiles")
                .join(rel)
        };
        if !path.exists() {
            return Err(AppError::NotFound(format!(
                "role template not found: {}",
                path.display()
            )));
        }
        Ok(std::fs::read_to_string(&path)?)
    }

    fn append_turn(
        &self,
        turn_id: &str,
        round: u32,
        role: Role,
        agent: AgentId,
        started: SystemTime,
        resp: &InvokeResponse,
    ) -> Result<()> {
        let ws = PathBuf::from(&self.task.workspace_path);
        let path = ws.join("meta").join("turns.jsonl");
        let started_ts = started
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let line = json!({
            "turn_id": turn_id,
            "round": round,
            "role": format!("{:?}", role),
            "agent": format!("{:?}", agent),
            "started_unix": started_ts,
            "duration_ms": resp.duration_ms,
            "exit_code": resp.exit_code,
            "raw_log_path": resp.raw_log_path,
            "artifacts": resp.artifacts_written,
            "stdout_len": resp.stdout.len(),
            "stderr_len": resp.stderr.len(),
        });
        let mut content = serde_json::to_string(&line)?;
        content.push('\n');
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        f.write_all(content.as_bytes())?;
        Ok(())
    }

    async fn transition(&mut self, next: FsmState) -> Result<()> {
        self.state = next;
        self.history.push(next.as_str().to_string());
        self.persist_state()?;
        if let Some(h) = &self.app_handle {
            // Tauri event — best-effort; we don't fail the FSM if emit fails.
            use tauri::Emitter;
            let _ = h.emit(
                "task.state_changed",
                json!({ "task_id": self.task.id, "state": next.as_str() }),
            );
        }
        Ok(())
    }

    fn persist_state(&self) -> Result<()> {
        let ws = PathBuf::from(&self.task.workspace_path);
        let path = ws.join("meta").join("state.json");
        let payload = StatePayload {
            state: self.state.as_str().to_string(),
            round: current_round(self.state),
            history: self.history.clone(),
            error: self.last_error.clone(),
        };
        std::fs::write(&path, serde_json::to_string_pretty(&payload)?)?;
        Ok(())
    }
}

fn role_short(r: Role) -> &'static str {
    match r {
        Role::Decider => "decide",
        Role::Executor => "execute",
        Role::Reviewer => "review",
    }
}

fn current_round(s: FsmState) -> u32 {
    match s {
        FsmState::Pending => 0,
        FsmState::R1Deciding | FsmState::R1Executing | FsmState::R1Reviewing => 1,
        FsmState::R2Deciding | FsmState::R2Executing | FsmState::R2Reviewing => 2,
        FsmState::R3Deciding | FsmState::R3Executing | FsmState::R3Reviewing => 3,
        FsmState::NeedsHuman | FsmState::Done | FsmState::Failed => 0,
    }
}

fn round_boundary(prev: FsmState, next: FsmState) -> bool {
    matches!(
        (prev, next),
        (FsmState::R1Reviewing, _) | (FsmState::R2Reviewing, _) | (FsmState::R3Reviewing, _)
    )
}

fn build_user_message(role: Role, round: u32, intent: &str) -> String {
    let role_str = match role {
        Role::Decider => "decider",
        Role::Executor => "executor",
        Role::Reviewer => "reviewer",
    };
    format!(
        "You are the {role_str} for round {round}. The user's intent is:\n\n{intent}\n\n\
         Read the role contract in your CLAUDE.md / AGENTS.md and any prior decisions/ and \
         execution/ files in this workspace, then produce the artifacts described there.",
    )
}

// Silence unused warnings if a binary is built without using Permission.
#[allow(dead_code)]
fn _permission_static_check(_: Permission) {}

/// Parse a reviewer verdict from a review file. The first non-empty,
/// non-whitespace line must start with `PASS` (case-insensitive) or
/// `FAIL:<reason>` (case-insensitive, colon required).
///
/// Returns `(pass, reason)`. `reason` is empty when `pass == true`.
fn parse_review_verdict(path: &std::path::Path) -> (bool, String) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return (false, "review file not found".into()),
    };
    let first = content
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty());
    let first = match first {
        Some(l) => l,
        None => return (false, "first line is not PASS / FAIL:<reason>".into()),
    };
    let upper = first.to_ascii_uppercase();
    if upper.starts_with("PASS") {
        return (true, String::new());
    }
    if upper.starts_with("FAIL:") {
        let reason = first[5..].trim().to_string();
        return (false, reason);
    }
    (false, "first line is not PASS / FAIL:<reason>".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{ProbeResult, Permission as Perm};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::path::PathBuf;

    /// A canned adapter used by the orchestrator-level tests. Never spawns
    /// anything; `invoke` is a no-op returning a successful empty response.
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

    fn fresh_workspace(tag: &str) -> PathBuf {
        let id = uuid::Uuid::new_v4().to_string();
        let ws = std::env::temp_dir().join(format!("flow-fsm-{tag}-{id}"));
        std::fs::create_dir_all(ws.join("meta")).expect("mkdir meta");
        std::fs::create_dir_all(ws.join("decisions")).expect("mkdir decisions");
        ws
    }

    fn write_review(ws: &std::path::Path, round: u32, body: &str) -> PathBuf {
        let p = ws.join("decisions").join(format!("{round:03}-review.md"));
        std::fs::write(&p, body).expect("write review");
        p
    }

    fn dummy_profile() -> Profile {
        // Use a hand-built minimal profile so we don't read disk.
        use crate::profile::{RoleSpec, SwapSpec};
        let spec = RoleSpec {
            agent: AgentId::Claude,
            template_path: "templates/dev-decider.md".into(),
            artifacts: vec![],
        };
        Profile {
            name: "dev".into(),
            description: String::new(),
            default_permission: Perm::ReadOnly,
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

    fn make_orch(ws: &std::path::Path) -> Orchestrator {
        let task = Task {
            id: uuid::Uuid::new_v4().to_string(),
            intent: "test".into(),
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

    // ─── parse_review_verdict ────────────────────────────────────────────────

    #[test]
    fn verdict_missing_file_returns_not_found() {
        let ws = fresh_workspace("verdict-missing");
        let p = ws.join("decisions").join("001-review.md");
        let (pass, reason) = parse_review_verdict(&p);
        assert!(!pass);
        assert!(reason.contains("not found"), "got: {reason}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn verdict_pass_first_line() {
        let ws = fresh_workspace("verdict-pass");
        let p = write_review(&ws, 1, "PASS\n\nlooks good\n");
        let (pass, reason) = parse_review_verdict(&p);
        assert!(pass);
        assert!(reason.is_empty());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn verdict_pass_with_trailing_notes() {
        let ws = fresh_workspace("verdict-pass-trail");
        let p = write_review(&ws, 1, "pass with optional trailing notes\n");
        let (pass, reason) = parse_review_verdict(&p);
        assert!(pass);
        assert!(reason.is_empty());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn verdict_fail_with_reason() {
        let ws = fresh_workspace("verdict-fail");
        let p = write_review(&ws, 1, "FAIL: needs error handling\nadditional notes\n");
        let (pass, reason) = parse_review_verdict(&p);
        assert!(!pass);
        assert!(reason.contains("needs error handling"), "got: {reason}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn verdict_junk_first_line() {
        let ws = fresh_workspace("verdict-junk");
        let p = write_review(&ws, 1, "hello world\n");
        let (pass, reason) = parse_review_verdict(&p);
        assert!(!pass);
        assert!(reason.contains("PASS") || reason.contains("FAIL"), "got: {reason}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    // ─── check_interventions ─────────────────────────────────────────────────

    #[tokio::test]
    async fn intervention_pause_returns_ok_false_when_unpaused_quickly() {
        let ws = fresh_workspace("interv-pause");
        let mut orch = make_orch(&ws);
        orch.state = FsmState::R1Deciding;
        // Drop a paused-true control.json, then immediately flip to paused=false
        // from a background task so the spin-loop exits in tests.
        std::fs::write(
            ws.join("meta").join("control.json"),
            serde_json::json!({"paused": true}).to_string(),
        )
        .unwrap();
        let ctrl = ws.join("meta").join("control.json");
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = std::fs::write(
                &ctrl,
                serde_json::json!({"paused": false}).to_string(),
            );
        });
        let aborted = orch.check_interventions().await.unwrap();
        assert!(!aborted, "pause must not return aborted=true");
        // State unchanged.
        assert_eq!(orch.state, FsmState::R1Deciding);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn intervention_abort_transitions_to_failed() {
        let ws = fresh_workspace("interv-abort");
        // Write a fresh state.json so persist_state's first write works.
        std::fs::write(
            ws.join("meta").join("state.json"),
            r#"{"state":"R1_Executing","round":1,"history":[]}"#,
        )
        .unwrap();
        let mut orch = make_orch(&ws);
        orch.state = FsmState::R1Executing;
        std::fs::write(ws.join("meta").join("abort.flag"), "x").unwrap();

        let aborted = orch.check_interventions().await.unwrap();
        assert!(aborted, "abort must return aborted=true");
        assert_eq!(orch.state, FsmState::Failed);
        assert_eq!(orch.last_error.as_deref(), Some("用户中止"));
        assert!(!ws.join("meta").join("abort.flag").exists(), "flag should be removed");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn intervention_retry_rewinds_reviewing_to_deciding() {
        let ws = fresh_workspace("interv-retry");
        std::fs::write(
            ws.join("meta").join("state.json"),
            r#"{"state":"R2_Reviewing","round":2,"history":[]}"#,
        )
        .unwrap();
        let mut orch = make_orch(&ws);
        orch.state = FsmState::R2Reviewing;
        std::fs::write(ws.join("meta").join("retry.flag"), "x").unwrap();

        let aborted = orch.check_interventions().await.unwrap();
        assert!(!aborted);
        assert_eq!(orch.state, FsmState::R2Deciding);
        assert!(!ws.join("meta").join("retry.flag").exists());
        let _ = std::fs::remove_dir_all(&ws);
    }
}
