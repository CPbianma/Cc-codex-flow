//! [I] Live settings demo for `workspaces_root_override`.
//!
//! Loads Settings, sets the override, saves, then proves
//! `workspace::create_workspace` lands the new task under the override.

use std::path::PathBuf;

use chrono::Utc;
use flow_lib::paths::{init_paths, paths};
use flow_lib::settings::Settings;
use flow_lib::store::task::Task;
use flow_lib::workspace;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_paths()?;
    println!("[I] settings_path: {}", paths().settings_path.display());

    let override_dir = PathBuf::from("D:/flow-workspaces-test");
    std::fs::create_dir_all(&override_dir)?;
    println!("[I] override dir created: {}", override_dir.display());

    let mut s = Settings::load()?;
    println!(
        "[I] BEFORE override: workspaces_root_override = {:?}",
        s.workspaces_root_override
    );
    s.workspaces_root_override = Some(override_dir.clone());
    s.save()?;

    let reloaded = Settings::load()?;
    println!(
        "[I] AFTER  override: workspaces_root_override = {:?}",
        reloaded.workspaces_root_override
    );

    // Now build a dummy Task and call create_workspace — its workspace_path
    // (the returned root) must live under the override dir.
    let now = Utc::now();
    let task = Task {
        id: "i-demo-9999-aaaa-bbbb-cccc-dddddddddddd".into(),
        intent: "settings override demo".into(),
        profile: "dev".into(),
        mode: "auto".into(),
        state: "Pending".into(),
        workspace_path: String::new(), // filled by create_workspace
        created_at: now,
        updated_at: now,
    };
    // Best-effort cleanup of any prior run.
    let _ = std::fs::remove_dir_all(override_dir.join(&task.id));
    let root = workspace::create_workspace(&task)?;
    println!("[I] create_workspace returned: {}", root.display());
    if root.starts_with(&override_dir) {
        println!("[I] PASS new workspace landed under override root");
    } else {
        println!("[I] FAIL new workspace NOT under override root");
        std::process::exit(1);
    }

    // Restore default so we don't poison the next demo / build.
    let mut s = Settings::load()?;
    s.workspaces_root_override = None;
    s.save()?;
    println!("[I] restored: workspaces_root_override = None");
    Ok(())
}
