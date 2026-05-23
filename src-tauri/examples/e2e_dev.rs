//! Headless end-to-end driver for the `dev` profile.
//!
//! Runs the full orchestrator FSM without spinning up the Tauri window,
//! exercising real Claude + Codex CLIs against a fresh workspace. Used as
//! transcript-visible evidence for the FSM ([J][M][L]) — see the parent
//! issue for what we're proving.
//!
//! Configuration via env vars (falls back to PATH lookup if unset):
//!   FLOW_CLAUDE_BIN  path to a Claude Code CLI binary
//!   FLOW_CODEX_BIN   path to a Codex CLI binary
//!
//! Runtime is capped at 6 minutes — past that we tear down whatever
//! state we observed and exit nonzero so the human-driver can see we
//! stopped because of the timeout, not a real failure.

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
        eprintln!("e2e_dev failed: {e}");
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
    // We do NOT call store::init() here — Task::insert/get won't be used.
    // Workspace creation only needs the on-disk layout.

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
    println!("[e2e_dev] claude = {:?}", claude_bin);
    println!("[e2e_dev] codex  = {:?}", codex_bin);

    let task = Task::new(
        "写一个把 markdown 转 docx 的 Python 脚本".to_string(),
        "dev".to_string(),
        "auto".to_string(),
        String::new(),
    );
    let ws_path: PathBuf = workspace::create_workspace(&task)?;
    println!("[e2e_dev] workspace = {}", ws_path.display());

    // Patch the task so the orchestrator points at the realised workspace.
    let mut task = task;
    task.workspace_path = ws_path.to_string_lossy().into_owned();

    let mut profile = Profile::load("dev")?;
    // For autonomous E2E we use FullAuto so neither CLI prompts for tool
    // permissions interactively (Claude --permission-mode bypassPermissions,
    // Codex -s danger-full-access).
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

    // Hard 25-minute cap.
    let outcome = tokio::time::timeout(Duration::from_secs(25 * 60), orch.run()).await;
    let timed_out = outcome.is_err();
    if timed_out {
        println!("[e2e_dev] WALL-CLOCK CAP HIT (>25 min) — printing best-effort state below");
    } else if let Ok(Err(e)) = outcome {
        println!("[e2e_dev] orchestrator returned error: {e}");
    }

    summarise(&ws_path, &orch);

    if timed_out {
        std::process::exit(2);
    }
    Ok(())
}

fn summarise(ws: &std::path::Path, orch: &Orchestrator) {
    println!("[e2e_dev] terminal state: {:?}", orch.state);
    println!("[e2e_dev] history       : {:?}", orch.history);
    println!("[e2e_dev] last_error    : {:?}", orch.last_error);

    let artifacts = ws.join("artifacts");
    println!("[e2e_dev] artifacts/    :");
    walk_dir(&artifacts, "  ");

    let decisions = ws.join("decisions");
    println!("[e2e_dev] decisions/    :");
    walk_dir(&decisions, "  ");

    let stream = ws.join("meta").join("turns").join("r1-execute.stream.jsonl");
    match std::fs::metadata(&stream) {
        Ok(m) => println!(
            "[e2e_dev] r1-execute.stream.jsonl size = {} bytes (path = {})",
            m.len(),
            stream.display()
        ),
        Err(e) => println!(
            "[e2e_dev] r1-execute.stream.jsonl MISSING ({e}) at {}",
            stream.display()
        ),
    }

    // Skim every stream file for MCP-related errors so we can flag them
    // without spamming the report.
    let turns_dir = ws.join("meta").join("turns");
    if let Ok(rd) = std::fs::read_dir(&turns_dir) {
        let mut hits: Vec<String> = Vec::new();
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&p) {
                for (i, line) in content.lines().enumerate() {
                    let l = line.to_ascii_lowercase();
                    if l.contains("mcp") && (l.contains("error") || l.contains("fail")) {
                        let snippet = if line.len() > 200 { &line[..200] } else { line };
                        hits.push(format!(
                            "  {}:{} {}",
                            p.file_name().unwrap_or_default().to_string_lossy(),
                            i + 1,
                            snippet
                        ));
                    }
                }
            }
        }
        if !hits.is_empty() {
            println!("[e2e_dev] MCP error/fail signals:");
            for h in hits.iter().take(20) {
                println!("{}", h);
            }
        } else {
            println!("[e2e_dev] no MCP error/fail signals found in stream files");
        }
    }
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
