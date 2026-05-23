import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Settings, api } from "../lib/tauri-api";

interface Props {
  open: boolean;
  onClose: () => void;
}

export function SettingsDialog({ open: isOpen, onClose }: Props) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [pickedPath, setPickedPath] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen) return;
    setError(null);
    setPickedPath(null);
    api
      .getSettings()
      .then(setSettings)
      .catch((e) => setError(String(e)));
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const pickDir = async () => {
    setError(null);
    try {
      const result = await open({
        directory: true,
        multiple: false,
        title: "选择 workspaces 根目录",
      });
      if (typeof result === "string") {
        setPickedPath(result);
      }
    } catch (e: any) {
      setError(String(e));
    }
  };

  const save = async () => {
    if (!pickedPath) return;
    setSaving(true);
    setError(null);
    try {
      const next = await api.setWorkspacesRoot(pickedPath);
      setSettings(next);
      setPickedPath(null);
      onClose();
    } catch (e: any) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const current = settings?.workspaces_root ?? "(未配置)";
  const displayPath = pickedPath ?? current;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>设置</h3>

        <label>Workspaces 根目录</label>
        <div
          style={{
            border: "1px solid var(--border-strong)",
            borderRadius: "var(--r-sm)",
            padding: "10px 12px",
            background: "var(--surface)",
            fontFamily: "var(--font-mono)",
            fontSize: 12,
            color: pickedPath ? "var(--ink)" : "var(--ink-dim)",
            wordBreak: "break-all",
            lineHeight: 1.55,
            minHeight: 38,
          }}
          title={displayPath}
        >
          {displayPath}
        </div>
        <div style={{ marginTop: 8, display: "flex", gap: 8 }}>
          <button onClick={pickDir} type="button">
            选择目录…
          </button>
          {pickedPath && (
            <button
              type="button"
              onClick={() => setPickedPath(null)}
              style={{ color: "var(--ink-dim)" }}
            >
              撤销
            </button>
          )}
        </div>
        <div
          className="dim"
          style={{ marginTop: 10, fontSize: 11, lineHeight: 1.55 }}
        >
          只影响以后新建的任务；已存在任务保留原 workspace 路径。
        </div>

        {settings && (
          <>
            <label style={{ marginTop: 20 }}>Claude CLI</label>
            <div
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: 11.5,
                color: "var(--ink-dim)",
                padding: "6px 0",
                wordBreak: "break-all",
              }}
            >
              {settings.claude_cli_path ?? "(未检测到)"}
            </div>
            <label>Codex CLI</label>
            <div
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: 11.5,
                color: "var(--ink-dim)",
                padding: "6px 0",
                wordBreak: "break-all",
              }}
            >
              {settings.codex_cli_path ?? "(未检测到)"}
            </div>
          </>
        )}

        {error && (
          <div
            className="err"
            style={{
              marginTop: 14,
              maxWidth: "none",
              whiteSpace: "normal",
              cursor: "default",
            }}
          >
            {error}
          </div>
        )}

        <div className="modal-actions">
          <button onClick={onClose}>关闭</button>
          <button
            className="primary"
            disabled={!pickedPath || saving}
            onClick={save}
          >
            {saving ? "保存中…" : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}
