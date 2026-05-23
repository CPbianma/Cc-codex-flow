use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

pub mod claude;
pub mod codex;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentId {
    Claude,
    Codex,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Decider,
    Executor,
    Reviewer,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    ReadOnly,
    Edit,
    FullAuto,
}

#[derive(Clone, Debug)]
pub struct InvokeRequest {
    pub workspace: PathBuf,
    pub role: Role,
    pub system_prompt: String,
    pub user_message: String,
    pub permission: Permission,
    pub mcp_config_path: Option<PathBuf>,
    pub turn_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct InvokeResponse {
    pub stdout: String,
    pub stderr: String,
    pub artifacts_written: Vec<PathBuf>,
    pub raw_log_path: Option<PathBuf>,
    pub exit_code: i32,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProbeResult {
    pub agent: AgentId,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
}

#[async_trait]
pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> AgentId;

    /// Quick health check — invoke `--version`.
    async fn probe(&self) -> ProbeResult;

    /// Full invocation. Phase 1 implementations may return Unimplemented.
    async fn invoke(&self, req: InvokeRequest) -> Result<InvokeResponse>;
}
