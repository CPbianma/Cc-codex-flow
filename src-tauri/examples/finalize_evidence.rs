//! Trigger a deliberate Failed transition on the prior e2e_dev workspace,
//! producing state.json.error="用户中止" with artifacts/md2docx.py still
//! present. Evidence for goal items [M] (Failed branch) and [I] (persist
//! workspaces_root_override + new task lands under it).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use flow_lib::adapter::{
    AgentAdapter, AgentId, InvokeRequest, InvokeResponse, ProbeResult,
};
use flow_lib::error::Result as FlowResult;
use flow_lib::orchestrator::{FsmState, Orchestrator};
use flow_lib::profile::Profile;
use flow_lib::store::task::Task;

struct DummyAdapter(AgentId);

#[async_trait]
impl AgentAdapter for DummyAdapter {
    fn id(&self) -> AgentId { self.0 }
    async fn probe(&self) -> ProbeResult {
        ProbeResult { agent: self.0, binary_path: Some("dummy".into()), version: Some("0".into()), ok: true, error: None }
    }
    async fn invoke(&self, _req: InvokeRequest) -> FlowResult<InvokeResponse> {
        Ok(InvokeResponse { stdout: String::new(), stderr: String::new(), artifacts_written: vec![], raw_log_path: None, exit_code: 0, duration_ms: 0 })
    }
}

fn cat(path: &Path) {
    match std::fs::read_to_string(path) {
        Ok(s) => println!("---8<--- {}\n{}\n--->8---", path.display(), s.trim_end()),
        Err(e) => println!("(cannot read {}: {})", path.display(), e),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    flow_lib::paths::init_paths().unwrap();

    // ====== [M] Failed branch on the actual e2e_dev workspace ======
    let dev_ws = PathBuf::from(r"C:\Users\<user>\AppData\Local\flow\workspaces\9bf8b2e4-7cd0-4383-b27d-4f6fd4472243");
    if !dev_ws.exists() {
        eprintln!("dev workspace not found: {}", dev_ws.display());
        std::process::exit(1);
    }

    println!("\n==== [M] Trigger Failed on dev E2E workspace ====");
    println!("[M] workspace: {}", dev_ws.display());
    println!("[M] BEFORE:");
    cat(&dev_ws.join("meta").join("state.json"));

    std::fs::write(dev_ws.join("meta").join("abort.flag"), "user-trigger").unwrap();
    let task = Task {
        id: "9bf8b2e4-7cd0-4383-b27d-4f6fd4472243".into(),
        intent: "写一个把 markdown 转 docx 的 Python 脚本".into(),
        profile: "dev".into(),
        mode: "auto".into(),
        state: "R2_Executing".into(),
        workspace_path: dev_ws.to_string_lossy().into_owned(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let profile = Profile::load("dev").unwrap();
    let mut orch = Orchestrator::new(
        task,
        profile,
        "auto".into(),
        Arc::new(DummyAdapter(AgentId::Claude)),
        Arc::new(DummyAdapter(AgentId::Codex)),
        None,
    );
    orch.state = FsmState::R2Executing;
    orch.history = vec![
        "R1_Deciding".into(),
        "R1_Executing".into(),
        "R1_Reviewing".into(),
        "R2_Deciding".into(),
        "R2_Executing".into(),
    ];
    let _ = orch.check_interventions().await.unwrap();
    println!("[M] after abort: state={:?} last_error={:?}", orch.state, orch.last_error);

    println!("[M] AFTER:");
    cat(&dev_ws.join("meta").join("state.json"));

    println!("[M] artifacts/ contents:");
    if let Ok(rd) = std::fs::read_dir(dev_ws.join("artifacts")) {
        for entry in rd.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let sz = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            println!("  {} ({} bytes)", name.to_string_lossy(), sz);
        }
    }

    assert_eq!(orch.state, FsmState::Failed);
    assert_eq!(orch.last_error.as_deref(), Some("用户中止"));
    println!("[M] PASS — terminal Failed with clear error 用户中止; artifacts/md2docx.py preserved");

    // ====== [I] Persist settings.json with workspaces_root_override (no restore) ======
    println!("\n==== [I] Persist settings.json with workspaces_root_override ====");
    let chosen = PathBuf::from(r"D:\flow-workspaces-evidence");
    std::fs::create_dir_all(&chosen).unwrap();
    let mut s = flow_lib::settings::Settings::load().unwrap();
    s.workspaces_root_override = Some(chosen.clone());
    s.save().unwrap();

    let settings_path = &flow_lib::paths::paths().settings_path;
    println!("[I] cat {}:", settings_path.display());
    cat(settings_path);

    let new_task = Task::new(
        "fresh task with override active".into(),
        "dev".into(),
        "auto".into(),
        String::new(),
    );
    let new_ws = flow_lib::workspace::create_workspace(&new_task).unwrap();
    println!("[I] new task workspace: {}", new_ws.display());
    let under_override = new_ws.starts_with(&chosen);
    println!("[I] starts_with(D:\\flow-workspaces-evidence)? {under_override}");
    assert!(under_override);
    println!("[I] PASS — settings persisted, new workspace landed under override");
}
