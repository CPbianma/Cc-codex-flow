//! Task Profile loader.
//!
//! Profile lookup order:
//!   1. user override at `<app_data>/profiles/<name>.toml`
//!   2. built-in at `<repo>/src-tauri/profiles/<name>.toml`
//!
//! We deliberately keep the TOML schema close to what the plan documents so
//! profile authors can hand-edit files without a separate spec.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::adapter::{AgentId, Permission};
use crate::error::{AppError, Result};
use crate::paths::paths;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoleSpec {
    pub agent: AgentId,
    pub template_path: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SwapSpec {
    pub decider: Option<AgentId>,
    pub executor: Option<AgentId>,
    pub reviewer: Option<AgentId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub default_permission: Permission,
    pub decider: RoleSpec,
    pub executor: RoleSpec,
    pub reviewer: RoleSpec,
    #[serde(default)]
    pub swap: SwapSpec,
}

impl Profile {
    /// Load a profile by name. Searches user dir first, then the bundled
    /// `src-tauri/profiles/` directory.
    pub fn load(name: &str) -> Result<Self> {
        let user_path = paths().profiles_user_dir.join(format!("{name}.toml"));
        if user_path.exists() {
            return Self::load_from(&user_path);
        }

        let builtin_path = builtin_dir().join(format!("{name}.toml"));
        if builtin_path.exists() {
            return Self::load_from(&builtin_path);
        }

        Err(AppError::NotFound(format!("profile '{name}'")))
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let p: Profile = toml::from_str(&raw)?;
        Ok(p)
    }
}

/// Best-effort lookup for the bundled profiles directory.
///
/// In development, this is `src-tauri/profiles` relative to `CARGO_MANIFEST_DIR`.
/// In a built binary, that env var still resolves at compile time, so the path
/// is baked in — fine for MVP. Production packaging will need a resource bundle.
fn builtin_dir() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest).join("profiles")
}
