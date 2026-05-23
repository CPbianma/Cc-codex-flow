//! Permission mapping for the two backing CLIs.
//!
//! Translates our internal [`Permission`] enum into the appropriate CLI
//! arguments / sandbox names for `claude` and `codex` respectively.

use crate::adapter::Permission;

/// Returns the `claude` CLI flags that realise the given permission level.
///
/// The slices contain individual argv entries — pass them straight to
/// `Command::args(...)`. No quoting is required at this layer; the OS does
/// argv splitting itself.
pub fn map_claude(p: Permission) -> Vec<&'static str> {
    match p {
        Permission::ReadOnly => vec![
            "--permission-mode",
            "plan",
            "--disallowedTools",
            "Edit,Write,Bash",
        ],
        Permission::Edit => vec!["--permission-mode", "acceptEdits"],
        Permission::FullAuto => vec!["--permission-mode", "bypassPermissions"],
    }
}

/// Returns the value to pass to `codex exec -s <...>`.
pub fn map_codex(p: Permission) -> &'static str {
    match p {
        Permission::ReadOnly => "read-only",
        Permission::Edit => "workspace-write",
        Permission::FullAuto => "danger-full-access",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_readonly_disallows_writing_tools() {
        let args = map_claude(Permission::ReadOnly);
        assert!(args.contains(&"plan"));
        assert!(args.contains(&"Edit,Write,Bash"));
    }

    #[test]
    fn claude_full_auto_bypasses() {
        assert_eq!(
            map_claude(Permission::FullAuto),
            vec!["--permission-mode", "bypassPermissions"]
        );
    }

    #[test]
    fn codex_mappings_are_stable() {
        assert_eq!(map_codex(Permission::ReadOnly), "read-only");
        assert_eq!(map_codex(Permission::Edit), "workspace-write");
        assert_eq!(map_codex(Permission::FullAuto), "danger-full-access");
    }
}
