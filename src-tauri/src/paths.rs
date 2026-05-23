use std::path::PathBuf;
use std::sync::OnceLock;

use crate::error::Result;

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub workspaces_root: PathBuf,
    pub db_path: PathBuf,
    pub profiles_user_dir: PathBuf,
    pub settings_path: PathBuf,
}

static PATHS: OnceLock<AppPaths> = OnceLock::new();

pub fn init_paths() -> Result<&'static AppPaths> {
    let base = dirs::data_local_dir()
        .ok_or_else(|| crate::error::AppError::Other("no data_local_dir".into()))?
        .join("flow");

    let paths = AppPaths {
        workspaces_root: base.join("workspaces"),
        db_path: base.join("tasks.sqlite"),
        profiles_user_dir: base.join("profiles"),
        settings_path: base.join("settings.json"),
        data_dir: base,
    };

    std::fs::create_dir_all(&paths.data_dir)?;
    std::fs::create_dir_all(&paths.workspaces_root)?;
    std::fs::create_dir_all(&paths.profiles_user_dir)?;

    let _ = PATHS.set(paths);
    Ok(PATHS.get().unwrap())
}

pub fn paths() -> &'static AppPaths {
    PATHS.get().expect("AppPaths not initialized; call init_paths() first")
}
