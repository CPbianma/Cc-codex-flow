use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::paths::paths;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub claude_cli_path: Option<PathBuf>,
    pub codex_cli_path: Option<PathBuf>,
    pub workspaces_root_override: Option<PathBuf>,
    pub default_profile: String,
    pub default_mode: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            claude_cli_path: which::which("claude").ok(),
            codex_cli_path: which::which("codex").ok(),
            workspaces_root_override: None,
            default_profile: "dev".into(),
            default_mode: "auto".into(),
        }
    }
}

impl Settings {
    pub fn load() -> Result<Self> {
        let path = &paths().settings_path;
        if !path.exists() {
            let s = Settings::default();
            s.save()?;
            return Ok(s);
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save(&self) -> Result<()> {
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(&paths().settings_path, raw)?;
        Ok(())
    }
}
