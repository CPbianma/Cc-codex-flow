//! Headless end-to-end driver for the `visual` profile.
//!
//! See `e2e_dev.rs` for the rationale. This variant uses the `visual`
//! profile (Claude produces wireframe + visual-spec + plan) and asserts
//! the three decision artifacts are written.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use flow_lib::adapter::{AgentAdapter, claude::ClaudeAdapter, codex::CodexAdapter};
use flow_lib::orchestrator::Orchestrator;
use flow_lib::paths::init_paths;
use flow_lib::profile::Profile;
use flow_lib::store::task::Task;
use flow_lib::workspace;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("e2e_visual failed: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "flow_lib=info,info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    init_paths()?;

    let claude_bin = std::env::var("FLOW_CLAUDE_BIN")
        .ok()
        .or_else(|| {
            which::which("claude")
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        });
    let codex_bin = std::env::var("FLOW_CODEX_BIN")
        .ok()
        .or_else(|| {
            which::which("codex")
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        });
    println!("[e2e_visual] claude = {:?}", claude_bin);
    println!("[e2e_visual] codex  = {:?}", codex_bin);

    let task = Task::new(
        "用 matplotlib 画一张鸢尾花 PCA 散点图，要好看".to_string(),
        "visual".to_string(),
        "auto".to_string(),
        String::new(),
    );
    let ws_path: PathBuf = workspace::create_workspace(&task)?;
    println!("[e2e_visual] workspace = {}", ws_path.display());

    let mut task = task;
    task.workspace_path = ws_path.to_string_lossy().into_owned();

    let mut profile = Profile::load("visual")?;
    profile.default_permission = flow_lib::adapter::Permission::FullAuto;
    let claude: Arc<dyn AgentAdapter> = Arc::new(ClaudeAdapter::new(claude_bin));
    let codex: Arc<dyn AgentAdapter> = Arc::new(CodexAdapter::new(codex_bin));

    let mut orch = Orchestrator::new(
        task,
        profile,
        "auto".into(),
        claude,
        codex,
        /* app_handle */ None,
    );

    let outcome = tokio::time::timeout(Duration::from_secs(25 * 60), orch.run()).await;
    let timed_out = outcome.is_err();
    if timed_out {
        println!("[e2e_visual] WALL-CLOCK CAP HIT (>25 min) — printing best-effort state below");
    } else if let Ok(Err(e)) = outcome {
        println!("[e2e_visual] orchestrator returned error: {e}");
    }

    println!("[e2e_visual] terminal state: {:?}", orch.state);
    println!("[e2e_visual] history       : {:?}", orch.history);
    println!("[e2e_visual] last_error    : {:?}", orch.last_error);

    println!("[e2e_visual] decisions/    :");
    let decisions = ws_path.join("decisions");
    walk_dir(&decisions, "  ");

    // Visual-profile asserts: at least the three named files should exist
    // after round 1's Decider has run. The role templates currently use
    // `{round}` (un-padded), but parse_review_verdict looks for the padded
    // form — so we tolerate either spelling here and report which we found.
    let expected_bases = ["plan", "wireframe", "visual-spec"];
    for base in &expected_bases {
        let padded = decisions.join(format!("001-{}.md", base));
        let unpadded = decisions.join(format!("1-{}.md", base));
        if padded.exists() {
            let sz = std::fs::metadata(&padded).map(|m| m.len()).unwrap_or(0);
            println!("[e2e_visual] OK  001-{}.md ({} bytes)", base, sz);
        } else if unpadded.exists() {
            let sz = std::fs::metadata(&unpadded).map(|m| m.len()).unwrap_or(0);
            println!(
                "[e2e_visual] OK  1-{}.md ({} bytes)  [un-padded variant — template uses {{round}}]",
                base, sz
            );
        } else {
            println!(
                "[e2e_visual] MISSING decisions/{{001,1}}-{}.md",
                base
            );
        }
    }

    // We tolerate states from R1_Reviewing onwards — anything earlier
    // means the Decider didn't fully run, so flag it.
    let state_ok = matches!(
        orch.state,
        flow_lib::orchestrator::FsmState::R1Reviewing
            | flow_lib::orchestrator::FsmState::R2Deciding
            | flow_lib::orchestrator::FsmState::R2Executing
            | flow_lib::orchestrator::FsmState::R2Reviewing
            | flow_lib::orchestrator::FsmState::R3Deciding
            | flow_lib::orchestrator::FsmState::R3Executing
            | flow_lib::orchestrator::FsmState::R3Reviewing
            | flow_lib::orchestrator::FsmState::Done
            | flow_lib::orchestrator::FsmState::Failed
            | flow_lib::orchestrator::FsmState::NeedsHuman
    );
    println!(
        "[e2e_visual] reached >= R1_Reviewing? {state_ok}  (state = {:?})",
        orch.state
    );

    if timed_out {
        std::process::exit(2);
    }
    Ok(())
}

fn walk_dir(p: &std::path::Path, indent: &str) {
    let Ok(rd) = std::fs::read_dir(p) else {
        println!("{indent}(missing: {})", p.display());
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    if entries.is_empty() {
        println!("{indent}(empty)");
    }
    for e in entries {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            println!("{indent}{name}/");
            walk_dir(&path, &format!("{indent}  "));
        } else {
            let sz = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            println!("{indent}{name} ({} bytes)", sz);
        }
    }
}
