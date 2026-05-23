//! Role-contract rendering & on-disk contract files.
//!
//! Each round, the orchestrator rewrites either `CLAUDE.md` or `AGENTS.md`
//! in the workspace so the corresponding CLI picks up the current role
//! contract automatically.

use std::collections::HashMap;
use std::path::Path;

use crate::adapter::AgentId;
use crate::error::Result;

/// Substitute `{key}` occurrences in `template` using `vars`.
///
/// Unknown keys are left intact so missing-context bugs surface visibly in
/// the rendered prompt rather than being silently dropped.
pub fn render_role(template: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end_rel) = template[i + 1..].find('}') {
                let end = i + 1 + end_rel;
                let key = &template[i + 1..end];
                if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                    if let Some(v) = vars.get(key) {
                        out.push_str(v);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        // Default: copy one char (UTF-8 safe)
        let ch_end = next_char_boundary(template, i);
        out.push_str(&template[i..ch_end]);
        i = ch_end;
    }
    out
}

fn next_char_boundary(s: &str, i: usize) -> usize {
    let mut j = i + 1;
    while j < s.len() && !s.is_char_boundary(j) {
        j += 1;
    }
    j
}

/// Write the per-round contract file the target agent will read.
///
/// * Claude → `<workspace>/CLAUDE.md`
/// * Codex  → `<workspace>/AGENTS.md`
pub fn write_contract(workspace: &Path, agent: AgentId, content: &str) -> Result<()> {
    let name = match agent {
        AgentId::Claude => "CLAUDE.md",
        AgentId::Codex => "AGENTS.md",
    };
    std::fs::write(workspace.join(name), content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_known_keys_and_keeps_unknown() {
        let mut vars = HashMap::new();
        vars.insert("name".into(), "Alice".into());
        let out = render_role("Hello {name}, missing={absent}, literal={", &vars);
        assert_eq!(out, "Hello Alice, missing={absent}, literal={");
    }

    #[test]
    fn handles_no_placeholders() {
        let out = render_role("plain text", &HashMap::new());
        assert_eq!(out, "plain text");
    }

    #[test]
    fn handles_utf8_safely() {
        let mut vars = HashMap::new();
        vars.insert("who".into(), "世界".into());
        let out = render_role("你好 {who}", &vars);
        assert_eq!(out, "你好 世界");
    }
}
