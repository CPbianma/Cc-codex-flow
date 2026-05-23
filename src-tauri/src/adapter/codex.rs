use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use crate::adapter::{AgentAdapter, AgentId, InvokeRequest, InvokeResponse, ProbeResult};
use crate::bridge::permission::map_codex;
use crate::error::{AppError, Result};

pub struct CodexAdapter {
    binary: String,
}

impl CodexAdapter {
    pub fn new(binary: Option<String>) -> Self {
        let binary = binary.unwrap_or_else(|| {
            which::which("codex")
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "codex".into())
        });
        Self { binary }
    }
}

#[async_trait]
impl AgentAdapter for CodexAdapter {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    async fn probe(&self) -> ProbeResult {
        let lower = self.binary.to_ascii_lowercase();
        let out = if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            Command::new("cmd.exe")
                .arg("/C")
                .arg(&self.binary)
                .arg("--version")
                .output()
                .await
        } else if lower.ends_with(".ps1") {
            Command::new("powershell.exe")
                .arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(&self.binary)
                .arg("--version")
                .output()
                .await
        } else {
            Command::new(&self.binary).arg("--version").output().await
        };

        match out {
            Ok(o) if o.status.success() => ProbeResult {
                agent: AgentId::Codex,
                binary_path: Some(self.binary.clone()),
                version: Some(String::from_utf8_lossy(&o.stdout).trim().to_string()),
                ok: true,
                error: None,
            },
            Ok(o) => ProbeResult {
                agent: AgentId::Codex,
                binary_path: Some(self.binary.clone()),
                version: None,
                ok: false,
                error: Some(format!(
                    "exit={:?} stderr={}",
                    o.status.code(),
                    String::from_utf8_lossy(&o.stderr)
                )),
            },
            Err(e) => ProbeResult {
                agent: AgentId::Codex,
                binary_path: Some(self.binary.clone()),
                version: None,
                ok: false,
                error: Some(e.to_string()),
            },
        }
    }

    async fn invoke(&self, req: InvokeRequest) -> Result<InvokeResponse> {
        let started_wall = SystemTime::now();
        let started = Instant::now();

        // Persist the system prompt to AGENTS.md so Codex picks it up via
        // both its built-in convention *and* the explicit -c flag below.
        let agents_md = req.workspace.join("AGENTS.md");
        std::fs::write(&agents_md, &req.system_prompt)?;

        let turns_dir = req.workspace.join("meta").join("turns");
        std::fs::create_dir_all(&turns_dir)?;
        let raw_log_path = turns_dir.join(format!("{}.stream.jsonl", req.turn_id));

        let sandbox = map_codex(req.permission);

        // Build argv. Note Codex uses `--` to separate the prompt from flags.
        let mut args: Vec<String> = Vec::new();
        args.push("exec".into());
        args.push("--skip-git-repo-check".into());
        args.push("-C".into());
        args.push(req.workspace.to_string_lossy().into_owned());
        args.push("-s".into());
        args.push(sandbox.into());
        args.push("-c".into());
        args.push(format!(
            "experimental_instructions_file={}",
            agents_md.to_string_lossy()
        ));

        // Inject MCP servers from <workspace>/mcp.shared.json (Claude-style
        // `mcpServers` map) as Codex `-c mcp_servers.<name>.…` overrides.
        // Silently skip if the file is missing or malformed.
        inject_mcp_servers(&req.workspace, &mut args);

        args.push("--".into());
        args.push(req.user_message.clone());

        tracing::info!(turn = %req.turn_id, binary = %self.binary, "spawning codex");

        // Windows refuses to spawn .cmd/.bat directly with arbitrary args
        // via CreateProcess. Detect and wrap through cmd.exe /C.
        let is_cmd = self
            .binary
            .to_ascii_lowercase()
            .ends_with(".cmd")
            || self.binary.to_ascii_lowercase().ends_with(".bat");
        let is_ps1 = self.binary.to_ascii_lowercase().ends_with(".ps1");

        let mut cmd = if is_cmd {
            let mut c = Command::new("cmd.exe");
            c.arg("/C").arg(&self.binary);
            for a in &args {
                c.arg(a);
            }
            c
        } else if is_ps1 {
            let mut c = Command::new("powershell.exe");
            c.arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(&self.binary);
            for a in &args {
                c.arg(a);
            }
            c
        } else {
            let mut c = Command::new(&self.binary);
            c.args(&args);
            c
        };

        let mut child = cmd
            .current_dir(&req.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AppError::Adapter(format!("failed to spawn codex: {e}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Adapter("codex stdout missing".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Adapter("codex stderr missing".into()))?;

        // Codex doesn't emit stream-json by default — just capture text and
        // mirror it to the log file line-by-line.
        let log_path_for_task = raw_log_path.clone();
        let stdout_task = tokio::spawn(async move {
            let mut log = match std::fs::File::create(&log_path_for_task) {
                Ok(f) => f,
                Err(e) => return Err(AppError::Io(e)),
            };
            use std::io::Write;
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut text = String::new();
            while let Some(line) = lines.next_line().await? {
                writeln!(log, "{}", line).ok();
                text.push_str(&line);
                text.push('\n');
            }
            Ok::<String, AppError>(text)
        });

        let stderr_task = tokio::spawn(async move {
            let mut buf = String::new();
            let mut reader = BufReader::new(stderr);
            reader.read_to_string(&mut buf).await.ok();
            buf
        });

        let status = child
            .wait()
            .await
            .map_err(|e| AppError::Adapter(format!("waiting on codex: {e}")))?;

        let final_text = match stdout_task.await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(AppError::Adapter(format!("stdout join: {e}"))),
        };
        let stderr_str = stderr_task.await.unwrap_or_default();

        let artifacts = collect_artifacts(&req.workspace, started_wall)?;

        Ok(InvokeResponse {
            stdout: final_text,
            stderr: stderr_str,
            artifacts_written: artifacts,
            raw_log_path: Some(raw_log_path),
            exit_code: status.code().unwrap_or(-1),
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }
}

fn collect_artifacts(workspace: &Path, since: SystemTime) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk_for_artifacts(workspace, workspace, since, &mut out)?;
    out.sort();
    Ok(out)
}

/// Read `<workspace>/mcp.shared.json` and translate its `mcpServers` map into
/// Codex `-c mcp_servers.<name>.<key>=<toml-value>` overrides appended to
/// `args`. Silently no-ops if the file is missing or invalid.
fn inject_mcp_servers(workspace: &Path, args: &mut Vec<String>) {
    let path = workspace.join("mcp.shared.json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return,
    };
    let root: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return,
    };
    let servers = match root.get("mcpServers").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return,
    };

    for (name, spec) in servers {
        let spec_obj = match spec.as_object() {
            Some(o) => o,
            None => continue,
        };

        if let Some(cmd) = spec_obj.get("command").and_then(|v| v.as_str()) {
            args.push("-c".into());
            args.push(format!(
                "mcp_servers.{}.command={}",
                name,
                toml_string(cmd)
            ));
        } else {
            // command is required for a usable mcp server entry
            continue;
        }

        if let Some(arr) = spec_obj.get("args").and_then(|v| v.as_array()) {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(toml_string))
                .collect();
            args.push("-c".into());
            args.push(format!(
                "mcp_servers.{}.args=[{}]",
                name,
                parts.join(", ")
            ));
        }

        if let Some(env_obj) = spec_obj.get("env").and_then(|v| v.as_object()) {
            for (k, v) in env_obj {
                if let Some(s) = v.as_str() {
                    args.push("-c".into());
                    args.push(format!(
                        "mcp_servers.{}.env.{}={}",
                        name,
                        k,
                        toml_string(s)
                    ));
                }
            }
        }
    }
}

/// Encode a Rust string as a TOML basic string literal: wrap in double quotes
/// and escape backslash, double-quote, and common control characters.
fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn walk_for_artifacts(
    root: &Path,
    dir: &Path,
    since: SystemTime,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().map(|n| n == "meta").unwrap_or(false)
                && path.parent() == Some(root)
            {
                continue;
            }
            walk_for_artifacts(root, &path, since, out)?;
        } else {
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    if mtime >= since {
                        out.push(path);
                    }
                }
            }
        }
    }
    Ok(())
}
