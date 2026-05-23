mod adapter;
mod bridge;
mod commands;
mod error;
mod orchestrator;
mod paths;
mod profile;
mod settings;
mod store;
mod workspace;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "flow_lib=debug,warn".into()),
        )
        .init();

    paths::init_paths().expect("failed to init app paths");
    store::init().expect("failed to init sqlite");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::create_task,
            commands::list_tasks,
            commands::get_task,
            commands::list_workspace_files,
            commands::read_workspace_file,
            commands::probe_agents,
            commands::get_settings,
            commands::set_workspaces_root,
            commands::reset_task,
            commands::start_task,
            commands::get_task_state,
            commands::intervene,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
