//! Phase-2 FSM driver.
//!
//! For Phase 2 the reviewer step always returns "pass" — see TODO below.
//! The full plan walks rounds R1 → R2 → R3 (with role swap) → R4 (human),
//! but until we wire real review parsing we just run R1 to completion in
//! auto mode.

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
        let req = InvokeRequest {
            workspace: workspace.clone(),
            role,
            system_prompt: rendered,
            user_message: build_user_message(role, round, &self.task.intent),
            permission: self.profile.default_permission,
            mcp_config_path: if mcp.exists() { Some(mcp) } else { None },
            turn_id: turn_id.clone(),
        };

        let started = SystemTime::now();
        let resp = adapter.invoke(req).await?;
        self.append_turn(&turn_id, round, role, agent_id, started, &resp)?;

        // TODO(phase-3): parse `decisions/{n}-review.md` for "Verdict: pass"
        // and feed it back. For now any reviewer step counts as pass so the
        // FSM can complete end-to-end during development.
        if matches!(role, Role::Reviewer) {
            return Ok(true);
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
