use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use crate::adapter::{AgentAdapter, AgentId, InvokeRequest, InvokeResponse, ProbeResult};
use crate::bridge::permission::map_claude;
use crate::error::{AppError, Result};

pub struct ClaudeAdapter {
    binary: String,
}

impl ClaudeAdapter {
    pub fn new(binary: Option<String>) -> Self {
        let binary = binary.unwrap_or_else(|| {
            which::which("claude")
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "claude".into())
        });
        Self { binary }
    }
}

#[async_trait]
impl AgentAdapter for ClaudeAdapter {
    fn id(&self) -> AgentId {
        AgentId::Claude
    }

    async fn probe(&self) -> ProbeResult {
        let out = Command::new(&self.binary).arg("--version").output().await;

        match out {
            Ok(o) if o.status.success() => ProbeResult {
                agent: AgentId::Claude,
                binary_path: Some(self.binary.clone()),
                version: Some(String::from_utf8_lossy(&o.stdout).trim().to_string()),
                ok: true,
                error: None,
            },
            Ok(o) => ProbeResult {
                agent: AgentId::Claude,
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
                agent: AgentId::Claude,
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

        let turns_dir = req.workspace.join("meta").join("turns");
        std::fs::create_dir_all(&turns_dir)?;
        let raw_log_path = turns_dir.join(format!("{}.stream.jsonl", req.turn_id));

        // Build argv. The order matches the spec in the Phase 2 plan.
        let mut args: Vec<String> = Vec::new();
        args.push("-p".into());
        args.push(req.user_message.clone());
        args.push("--add-dir".into());
        args.push(req.workspace.to_string_lossy().into_owned());
        args.push("--output-format".into());
        args.push("stream-json".into());
        args.push("--include-partial-messages".into());
        args.push("--verbose".into());

        for a in map_claude(req.permission) {
            args.push(a.to_string());
        }

        if !req.system_prompt.is_empty() {
            args.push("--system-prompt".into());
            args.push(req.system_prompt.clone());
        }

        if let Some(mcp) = &req.mcp_config_path {
            if mcp.exists() {
                args.push("--mcp-config".into());
                args.push(mcp.to_string_lossy().into_owned());
            }
        }

        tracing::info!(turn = %req.turn_id, binary = %self.binary, "spawning claude");

        let mut child = Command::new(&self.binary)
            .args(&args)
            .current_dir(&req.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AppError::Adapter(format!("failed to spawn claude: {e}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Adapter("claude stdout missing".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Adapter("claude stderr missing".into()))?;

        // Spawn a task that copies stdout to the log file AND parses
        // stream-json into a final text string in parallel.
        let log_path_for_task = raw_log_path.clone();
        let stdout_task = tokio::spawn(async move {
            let mut log = match std::fs::File::create(&log_path_for_task) {
                Ok(f) => f,
                Err(e) => return Err(AppError::Io(e)),
            };
            use std::io::Write;
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut final_text = String::new();
            while let Some(line) = lines.next_line().await? {
                // Mirror to log file first (best-effort).
                writeln!(log, "{}", line).ok();

                // Parse the line as JSON; ignore non-JSON lines.
                let Ok(value): std::result::Result<serde_json::Value, _> =
                    serde_json::from_str(&line)
                else {
                    continue;
                };
                extract_text_from_event(&value, &mut final_text);
            }
            Ok::<String, AppError>(final_text)
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
            .map_err(|e| AppError::Adapter(format!("waiting on claude: {e}")))?;

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

/// Walk a single stream-json event and append any text content it contributes
/// to `final_text`.
///
/// We accept a few shapes Claude is known to emit; unknown shapes are simply
/// skipped so a CLI update can't break the orchestrator silently.
fn extract_text_from_event(v: &serde_json::Value, final_text: &mut String) {
    let event_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match event_type {
        // Final assistant message blob. Some claude-code versions emit the
        // entire result text in `result` or `message.content[*].text`.
        "result" | "message_stop" | "assistant" | "message" => {
            if let Some(s) = v.get("result").and_then(|x| x.as_str()) {
                if !s.is_empty() {
                    final_text.push_str(s);
                    return;
                }
            }
            if let Some(msg) = v.get("message") {
                push_content_text(msg, final_text);
            }
        }
        "content_block_delta" => {
            if let Some(delta) = v.get("delta") {
                if let Some(t) = delta.get("text").and_then(|x| x.as_str()) {
                    final_text.push_str(t);
                }
            }
        }
        _ => {}
    }
}

fn push_content_text(msg: &serde_json::Value, out: &mut String) {
    if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
        for block in arr {
            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                    out.push_str(t);
                }
            }
        }
    } else if let Some(s) = msg.get("content").and_then(|c| c.as_str()) {
        out.push_str(s);
    }
}

/// Recursively walk `workspace`, returning files whose mtime is at least
/// `since`. The `meta/` subtree is skipped — we don't want our own log files
/// to show up as agent-produced artifacts.
fn collect_artifacts(workspace: &Path, since: SystemTime) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk_for_artifacts(workspace, workspace, since, &mut out)?;
    out.sort();
    Ok(out)
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
            // Skip the meta subtree at any depth from root.
            if path.strip_prefix(root).ok() == Some(Path::new("meta"))
                || path.file_name().map(|n| n == "meta").unwrap_or(false)
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
