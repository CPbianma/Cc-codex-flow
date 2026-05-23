import { useEffect, useMemo, useState } from "react";
import { Task, api } from "../lib/tauri-api";
import { DiffView, MarkdownView } from "../lib/markdown";

interface Props {
  task: Task | null;
  onNewTask?: () => void;
}

interface FsmState {
  state: string;
  round?: number;
  history?: string[];
  error?: string;
}

interface TurnRecord {
  turn_id: string;
  round: number;
  role: string;
  agent: string;
  started_unix: number;
  duration_ms: number;
  exit_code: number | null;
  raw_log_path: string | null;
  artifacts: string[];
  stdout_len?: number;
  stderr_len?: number;
}

interface LoadedArtifact {
  relativePath: string;
  content: string;
  error?: string;
}

const ROLE_LABEL: Record<string, string> = {
  Decider: "决策",
  Executor: "执行",
  Reviewer: "审查",
};

function agentClass(agent: string): string {
  return agent.toLowerCase() === "codex" ? "agent-codex" : "agent-claude";
}

/** Strip the task workspace prefix from an absolute artifact path so we can
 *  hand a workspace-relative path to `read_workspace_file`. Falls back to
 *  the original string if it's already relative. */
function toRelative(absOrRel: string, workspace: string): string {
  // Normalize separators on both sides.
  const norm = absOrRel.replace(/\\/g, "/");
  const ws = workspace.replace(/\\/g, "/").replace(/\/+$/, "");
  if (norm.startsWith(ws + "/")) return norm.slice(ws.length + 1);
  // Some adapters report paths relative already.
  return norm.replace(/^\.\//, "");
}

function fmtDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.floor((ms % 60_000) / 1000);
  return `${m}m${s}s`;
}

function pickRenderer(rel: string): "md" | "diff" | "text" {
  const lower = rel.toLowerCase();
  if (lower.endsWith(".md") || lower.endsWith(".markdown")) return "md";
  if (lower.endsWith(".patch") || lower.endsWith(".diff")) return "diff";
  return "text";
}

export function ThreadView({ task, onNewTask }: Props) {
  const [fsm, setFsm] = useState<FsmState | null>(null);
  const [turns, setTurns] = useState<TurnRecord[]>([]);
  const [artifacts, setArtifacts] = useState<Record<string, LoadedArtifact>>(
    {}
  );
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [feedbackOpen, setFeedbackOpen] = useState(false);
  const [feedbackText, setFeedbackText] = useState("");
  const [toast, setToast] = useState<{ msg: string; kind: "ok" | "err" } | null>(
    null
  );
  const [streamTail, setStreamTail] = useState<string | null>(null);

  // Auto-dismiss toast after 2500ms.
  useEffect(() => {
    if (!toast) return;
    const h = setTimeout(() => setToast(null), 2500);
    return () => clearTimeout(h);
  }, [toast]);

  // Reset transient UI when switching task.
  useEffect(() => {
    setFeedbackOpen(false);
    setFeedbackText("");
    setError(null);
    setArtifacts({});
    setTurns([]);
    setFsm(null);
    setStreamTail(null);
    setToast(null);
  }, [task?.id]);

  // Poll state.json + turns.jsonl every 1.5s.
  useEffect(() => {
    if (!task) return;
    let cancelled = false;

    const tick = async () => {
      // FSM state.
      try {
        const raw = await api.getTaskState(task.id);
        if (cancelled) return;
        try {
          setFsm(JSON.parse(raw));
        } catch {
          setFsm({ state: raw });
        }
      } catch {
        if (!cancelled) setFsm(null);
      }

      // Turns log.
      try {
        const raw = await api.readWorkspaceFile(task.id, "meta/turns.jsonl");
        if (cancelled) return;
        const parsed: TurnRecord[] = raw
          .split("\n")
          .map((l) => l.trim())
          .filter(Boolean)
          .map((l) => {
            try {
              return JSON.parse(l) as TurnRecord;
            } catch {
              return null;
            }
          })
          .filter((x): x is TurnRecord => x != null);
        setTurns(parsed);
      } catch {
        if (!cancelled) setTurns([]);
      }
    };
    tick();
    const h = setInterval(tick, 1500);
    return () => {
      cancelled = true;
      clearInterval(h);
    };
  }, [task?.id]);

  // Load artifact contents as turns appear. We keep a cache keyed by
  // `<turn_id>::<rel>` so we don't re-read files on every poll.
  useEffect(() => {
    if (!task) return;
    let cancelled = false;

    const needed: Array<{ turnId: string; rel: string }> = [];
    for (const t of turns) {
      for (const a of t.artifacts ?? []) {
        const rel = toRelative(a, task.workspace_path);
        const key = `${t.turn_id}::${rel}`;
        if (artifacts[key] == null) needed.push({ turnId: t.turn_id, rel });
      }
    }
    if (needed.length === 0) return;

    (async () => {
      const next: Record<string, LoadedArtifact> = {};
      for (const { turnId, rel } of needed) {
        const key = `${turnId}::${rel}`;
        try {
          const content = await api.readWorkspaceFile(task.id, rel);
          next[key] = { relativePath: rel, content };
        } catch (e: any) {
          next[key] = {
            relativePath: rel,
            content: "",
            error: String(e),
          };
        }
      }
      if (!cancelled && Object.keys(next).length > 0) {
        setArtifacts((prev) => ({ ...prev, ...next }));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [turns, task?.id, task?.workspace_path]);

  const currentState = fsm?.state ?? task?.state ?? "Pending";
  const isPending = currentState === "Pending" || currentState === "pending";
  const isTerminal =
    currentState === "Done" ||
    currentState === "Failed" ||
    currentState === "NeedsHuman";
  const isRunning = !isPending && !isTerminal;
  // Match FSM names like R1_Deciding / R1_Executing / R1_Reviewing.
  const phase = useMemo(() => {
    const m = /^R\d_(Deciding|Executing|Reviewing)$/.exec(currentState);
    return m ? m[1] : null;
  }, [currentState]);

  // Detect whether the latest in-flight turn already has a record.
  const latestTurnSuffix = useMemo(() => {
    if (!phase || !fsm?.round) return null;
    const map: Record<string, string> = {
      Deciding: "decide",
      Executing: "execute",
      Reviewing: "review",
    };
    return `r${fsm.round}-${map[phase]}`;
  }, [phase, fsm?.round]);

  const streamingPending = useMemo(() => {
    if (!latestTurnSuffix) return false;
    return !turns.some((t) => t.turn_id === latestTurnSuffix);
  }, [latestTurnSuffix, turns]);

  // Poll the in-flight turn's streaming jsonl tail.
  useEffect(() => {
    if (!task || !streamingPending || !latestTurnSuffix) {
      setStreamTail(null);
      return;
    }
    let cancelled = false;
    const path = `meta/turns/${latestTurnSuffix}.stream.jsonl`;
    const tick = async () => {
      try {
        const raw = await api.readWorkspaceFile(task.id, path);
        if (cancelled) return;
        // Keep only the last 4KB.
        const tail = raw.length > 4096 ? raw.slice(raw.length - 4096) : raw;
        setStreamTail(tail);
      } catch {
        if (!cancelled) setStreamTail(null);
      }
    };
    tick();
    const h = setInterval(tick, 1500);
    return () => {
      cancelled = true;
      clearInterval(h);
    };
  }, [task?.id, streamingPending, latestTurnSuffix]);

  if (!task) {
    return (
      <div className="pane pane-center">
        <div className="pane-body">
          <div className="thread-hero">
            <div className="thread-hero-inner">
              <div className="thread-hero-mark" aria-hidden />
              <h1>欢迎使用 Flow</h1>
              <p>
                在左侧新建任务，让 Claude 与 Codex 一起把它跑完。
                <br />
                所有产物会落在独立的 workspace 目录里。
              </p>

              <div
                className="prompt-card"
                onClick={onNewTask}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") onNewTask?.();
                }}
              >
                <div className="prompt-placeholder">
                  描述你的意图，例如「写一个把 markdown 转 docx 的 Python 脚本」…
                </div>
                <div className="prompt-actions">
                  <span className="prompt-hint">↵ 创建任务</span>
                  <button
                    className="btn-mini primary"
                    onClick={(e) => {
                      e.stopPropagation();
                      onNewTask?.();
                    }}
                  >
                    新建任务
                  </button>
                </div>
              </div>

              <div className="hero-tips">
                <div className="hero-tip" onClick={onNewTask}>
                  <strong>dev profile</strong>
                  让两个 agent 协作编写并审阅代码
                </div>
                <div className="hero-tip" onClick={onNewTask}>
                  <strong>visual profile</strong>
                  生成与迭代视觉资产（图像、SVG 等）
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    );
  }

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      await api.startTask(task.id);
      setToast({ msg: "已启动任务", kind: "ok" });
    } catch (e: any) {
      setError(String(e));
      setToast({ msg: `启动失败：${String(e)}`, kind: "err" });
    } finally {
      setStarting(false);
    }
  };

  const ACTION_TOAST: Record<string, string> = {
    pause: "已发送暂停信号",
    resume: "已发送继续信号",
    retry: "已发送重试信号",
    abort: "已发送中止信号",
  };

  const runIntervene = async (action: string) => {
    try {
      await api.intervene(task.id, action);
      const key = action.startsWith("feedback:") ? "feedback" : action;
      const msg =
        key === "feedback" ? "已发送反馈" : ACTION_TOAST[key] ?? `已发送：${action}`;
      setToast({ msg, kind: "ok" });
    } catch (e: any) {
      setError(String(e));
      setToast({ msg: `操作失败：${String(e)}`, kind: "err" });
    }
  };

  const submitFeedback = async () => {
    const text = feedbackText.trim();
    if (!text) return;
    await runIntervene(`feedback:${text}`);
    setFeedbackText("");
    setFeedbackOpen(false);
  };

  const reset = async () => {
    const ok = window.confirm(
      "确定要重置该任务吗？保留 decisions/execution/artifacts，但清空 FSM 状态。"
    );
    if (!ok) return;
    try {
      await api.resetTask(task.id);
      setArtifacts({});
      setTurns([]);
      setFsm({ state: "Pending", round: 0, history: [] });
      setToast({ msg: "已重置", kind: "ok" });
    } catch (e: any) {
      setError(String(e));
      setToast({ msg: `重置失败：${String(e)}`, kind: "err" });
    }
  };

  // Decide which action buttons to show.
  const showPause = isRunning && task.mode === "auto";
  const showResume = currentState === "NeedsHuman";
  const showRetry = phase === "Executing" || phase === "Reviewing";
  const showFeedback = isRunning || currentState === "NeedsHuman";
  // Reset is always available — pending, running, or terminal.
  const showReset = true;
  const showAbort = isRunning;

  return (
    <div className="pane pane-center">
      {toast && (
        <div className={`flow-toast ${toast.kind === "err" ? "err" : "ok"}`}>
          {toast.msg}
        </div>
      )}
      <div className="pane-header">
        <div className="thread-title">
          <span className="badge state">{currentState}</span>
          {fsm?.round != null && fsm.round > 0 && (
            <span className="badge">R{fsm.round}</span>
          )}
          {fsm?.error && (
            <span
              className="badge err-badge"
              title={fsm.error}
            >
              错误: {fsm.error.slice(0, 40)}
            </span>
          )}
          <span className="thread-title-text" title={task.id}>
            {task.intent}
          </span>
          <div
            className="thread-actions"
            style={{ marginLeft: "auto", display: "flex", gap: 6 }}
          >
            {isPending && (
              <button
                className="btn-mini primary"
                disabled={starting}
                onClick={start}
                title="启动任务（开始 R1：决策 → 执行 → 审查）"
              >
                {starting ? "启动中…" : "▶ 启动"}
              </button>
            )}
            {showPause && (
              <button
                className="btn-mini"
                onClick={() => runIntervene("pause")}
                title="暂停 FSM（写入 meta/control.json）"
              >
                暂停
              </button>
            )}
            {showResume && (
              <button
                className="btn-mini"
                onClick={() => runIntervene("resume")}
                title="清除暂停标志，让 FSM 继续运行"
              >
                继续
              </button>
            )}
            {showRetry && (
              <button
                className="btn-mini"
                onClick={() => runIntervene("retry")}
                title="重试当前轮（写入 meta/retry.flag）"
              >
                重试当前轮
              </button>
            )}
            {showFeedback && (
              <button
                className="btn-mini"
                onClick={() => setFeedbackOpen((v) => !v)}
                title="为下一轮添加反馈（写入 meta/feedback.jsonl）"
              >
                给下一轮反馈…
              </button>
            )}
            {showReset && (
              <button
                className="btn-mini"
                onClick={reset}
                title="重置为 Pending（保留 decisions/execution/artifacts，仅清空 FSM 状态）"
              >
                重置为 Pending
              </button>
            )}
            {showAbort && (
              <button
                className="btn-mini"
                onClick={() => runIntervene("abort")}
                title="中止当前任务（写入 meta/control.json abort=true）"
              >
                中止
              </button>
            )}
          </div>
        </div>
      </div>

      <div className="pane-body">
        <div className="thread-body">
          <div className="turn-card">
            <div className="turn-head">
              <div className="turn-avatar user" aria-hidden>
                {(task.profile || "?").slice(0, 1).toUpperCase()}
              </div>
              <span style={{ fontWeight: 500, color: "var(--ink)" }}>你</span>
              <span className="badge">created</span>
              <span className="dim" style={{ marginLeft: "auto" }}>
                {task.created_at}
              </span>
            </div>
            <pre className="turn-body">{task.intent}</pre>
          </div>

          {feedbackOpen && (
            <div className="turn-card" style={{ marginTop: 12 }}>
              <div className="turn-head">
                <span style={{ fontWeight: 500 }}>给下一轮的反馈</span>
              </div>
              <div style={{ padding: 12 }}>
                <textarea
                  autoFocus
                  value={feedbackText}
                  onChange={(e) => setFeedbackText(e.target.value)}
                  rows={4}
                  placeholder="写下你希望下一轮注意的点…（FSM 会在下一轮读取 meta/feedback.jsonl）"
                  style={{
                    width: "100%",
                    background: "var(--surface)",
                    color: "var(--ink)",
                    border: "1px solid var(--border-strong)",
                    borderRadius: "var(--r-sm)",
                    padding: "10px 12px",
                    fontFamily: "inherit",
                    fontSize: 13,
                    lineHeight: 1.55,
                    resize: "vertical",
                    outline: "none",
                  }}
                  onKeyDown={(e) => {
                    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                      submitFeedback();
                    }
                  }}
                />
                <div
                  style={{
                    display: "flex",
                    gap: 8,
                    justifyContent: "flex-end",
                    marginTop: 8,
                  }}
                >
                  <button
                    className="btn-mini"
                    onClick={() => {
                      setFeedbackOpen(false);
                      setFeedbackText("");
                    }}
                  >
                    取消
                  </button>
                  <button
                    className="btn-mini primary"
                    disabled={!feedbackText.trim()}
                    onClick={submitFeedback}
                  >
                    提交反馈
                  </button>
                </div>
              </div>
            </div>
          )}

          {error && (
            <div className="err" style={{ margin: "12px 0", maxWidth: "none" }}>
              {error}
            </div>
          )}
          {fsm?.error && (
            <div
              className="err"
              style={{
                margin: "12px 0",
                maxWidth: "none",
                whiteSpace: "normal",
              }}
            >
              FSM 错误：{fsm.error}
            </div>
          )}

          {turns.map((t) => {
            const roleLabel = ROLE_LABEL[t.role] ?? t.role;
            const tone = agentClass(t.agent);
            return (
              <div
                key={t.turn_id}
                className={`turn-card ${tone}`}
                style={{ marginTop: 12 }}
              >
                <div className="turn-head">
                  <span className={`turn-avatar ${tone}`} aria-hidden>
                    {t.agent.slice(0, 1).toUpperCase()}
                  </span>
                  <span style={{ fontWeight: 500, color: "var(--ink)" }}>
                    {t.agent}
                  </span>
                  <span className="badge">R{t.round}</span>
                  <span className="badge">{roleLabel}</span>
                  <span
                    className="badge"
                    style={{
                      background:
                        t.exit_code === 0 || t.exit_code == null
                          ? "var(--ok-soft)"
                          : "var(--fail-soft)",
                      color:
                        t.exit_code === 0 || t.exit_code == null
                          ? "var(--ok)"
                          : "var(--fail)",
                      borderColor: "transparent",
                    }}
                  >
                    exit {t.exit_code ?? "—"}
                  </span>
                  <span className="dim" style={{ marginLeft: "auto" }}>
                    {fmtDuration(t.duration_ms)}
                  </span>
                </div>
                <div className="turn-body" style={{ padding: 0 }}>
                  {(t.artifacts ?? []).length === 0 && (
                    <div
                      className="empty"
                      style={{
                        padding: "20px 16px",
                        textAlign: "left",
                      }}
                    >
                      该轮未产出工件。
                    </div>
                  )}
                  {(t.artifacts ?? []).map((abs) => {
                    const rel = toRelative(abs, task.workspace_path);
                    const key = `${t.turn_id}::${rel}`;
                    const a = artifacts[key];
                    const kind = pickRenderer(rel);
                    return (
                      <div key={key} className="artifact-block">
                        <div className="artifact-head">
                          <span className="artifact-kind">{kind}</span>
                          <code title={abs}>{rel}</code>
                        </div>
                        {a == null && (
                          <div
                            className="empty"
                            style={{ padding: "14px 16px" }}
                          >
                            加载中…
                          </div>
                        )}
                        {a?.error && (
                          <div
                            className="err"
                            style={{
                              margin: "8px 16px",
                              maxWidth: "none",
                              whiteSpace: "normal",
                            }}
                          >
                            读取失败：{a.error}
                          </div>
                        )}
                        {a && !a.error && kind === "md" && (
                          <div className="artifact-md">
                            <MarkdownView source={a.content} />
                          </div>
                        )}
                        {a && !a.error && kind === "diff" && (
                          <DiffView source={a.content} />
                        )}
                        {a && !a.error && kind === "text" && (
                          <pre className="artifact-text">{a.content}</pre>
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            );
          })}

          {streamingPending && (
            <div
              className="turn-card streaming"
              style={{ marginTop: 12 }}
            >
              <div className="turn-head">
                <span
                  className="turn-avatar system"
                  aria-hidden
                  style={{ background: "var(--ink-dim)" }}
                >
                  ⋯
                </span>
                <span style={{ fontWeight: 500 }}>{currentState}</span>
                <span className="dim">流式输出中…</span>
              </div>
              {streamTail ? (
                <pre className="streaming-tail">{streamTail}</pre>
              ) : (
                <div className="turn-body streaming-body">
                  Agent 正在工作，工件还未落盘。完成后会自动出现在这里。
                </div>
              )}
            </div>
          )}

          {fsm?.history && fsm.history.length > 0 && (
            <details
              className="turn-card"
              style={{ marginTop: 12 }}
            >
              <summary
                className="turn-head"
                style={{ cursor: "pointer", listStyle: "none" }}
              >
                <span style={{ fontWeight: 500 }}>FSM 历史</span>
                <span className="dim" style={{ marginLeft: "auto" }}>
                  {fsm.history.length} 步
                </span>
              </summary>
              <pre className="turn-body">{fsm.history.join("\n→ ")}</pre>
            </details>
          )}

          {isPending && (
            <div className="empty" style={{ marginTop: 16 }}>
              点击右上角「▶ 启动」开始 R1 — Claude 决策 → Codex 执行 → Claude
              审查。
            </div>
          )}
          {!fsm && !isPending && (
            <div className="empty" style={{ marginTop: 16 }}>
              等待 FSM 状态…
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
