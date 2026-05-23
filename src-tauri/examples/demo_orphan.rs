//! [E] Live orphan-task-recovery demo.
//!
//! Creates a workspace directory under the real flow data dir with a UUID
//! prefixed by `390b0f00`, writes a non-terminal `meta/state.json`, inserts
//! a matching `Task` row into the sqlite store, then calls
//! `flow_lib::recover_orphan_tasks()` and prints the count.

use std::path::PathBuf;

use chrono::Utc;
use flow_lib::paths::{init_paths, paths};
use flow_lib::store::{self, task::Task};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_paths()?;
    store::init()?;

    let task_id = "390b0f00-orphan-demo-task-id-aaaaaaaaaaaa";
    let ws: PathBuf = paths().workspaces_root.join(task_id);
    let meta = ws.join("meta");
    std::fs::create_dir_all(&meta)?;

    let state_path = meta.join("state.json");
    std::fs::write(
        &state_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "state": "R1_Executing",
            "round": 1,
            "history": ["R1_Deciding", "R1_Executing"],
        }))?,
    )?;

    // Insert a Task row pointing at the workspace. Build it by hand so the
    // id matches the prefix the goal calls out.
    let now = Utc::now();
    let task = Task {
        id: task_id.into(),
        intent: "orphan demo".into(),
        profile: "dev".into(),
        mode: "auto".into(),
        state: "R1_Executing".into(),
        workspace_path: ws.to_string_lossy().into_owned(),
        created_at: now,
        updated_at: now,
    };
    // Idempotent: best-effort insert; ignore "already exists" so the demo
    // can be re-run without manual cleanup.
    if let Err(e) = task.insert() {
        eprintln!("note: insert failed (likely duplicate, continuing): {e}");
    }

    println!("[E] workspace: {}", ws.display());
    println!("[E] state.json path: {}", state_path.display());

    let recovered = flow_lib::recover_orphan_tasks()?;
    println!("[E] recover_orphan_tasks() returned: {recovered}");

    // Re-read to prove the file actually mutated.
    let after = std::fs::read_to_string(&state_path)?;
    let v: serde_json::Value = serde_json::from_str(&after)?;
    let state = v["state"].as_str().unwrap_or("?");
    let err = v["error"].as_str().unwrap_or("?");
    if state == "Failed" && err == "孤儿任务" {
        println!("[E] PASS state=Failed error=孤儿任务");
    } else {
        println!("[E] FAIL state={state} error={err}");
        std::process::exit(1);
    }

    Ok(())
}
