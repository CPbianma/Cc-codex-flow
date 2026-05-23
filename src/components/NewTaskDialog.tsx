import { useEffect, useState } from "react";

interface Props {
  open: boolean;
  onClose: () => void;
  onSubmit: (input: { intent: string; profile: string; mode: string }) => void;
}

export function NewTaskDialog({ open, onClose, onSubmit }: Props) {
  const [intent, setIntent] = useState("");
  const [profile, setProfile] = useState("dev");
  const [mode, setMode] = useState("auto");

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const canSubmit = intent.trim().length > 0;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>新建任务</h3>
        <label>意图（intent）</label>
        <textarea
          autoFocus
          value={intent}
          onChange={(e) => setIntent(e.target.value)}
          rows={5}
          placeholder="例如：写一个把 markdown 转 docx 的 Python 脚本"
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && canSubmit) {
              onSubmit({ intent: intent.trim(), profile, mode });
              setIntent("");
            }
          }}
        />
        <div className="row" style={{ marginTop: 14 }}>
          <div>
            <label>Profile</label>
            <select value={profile} onChange={(e) => setProfile(e.target.value)}>
              <option value="dev">dev（代码）</option>
              <option value="visual">visual（视觉）</option>
            </select>
          </div>
          <div>
            <label>模式</label>
            <select value={mode} onChange={(e) => setMode(e.target.value)}>
              <option value="auto">(a) 全自动</option>
              <option value="checkpoint">(c) 关键节点暂停</option>
            </select>
          </div>
        </div>
        <div className="modal-actions">
          <button onClick={onClose}>取消</button>
          <button
            className="primary"
            disabled={!canSubmit}
            onClick={() => {
              onSubmit({ intent: intent.trim(), profile, mode });
              setIntent("");
            }}
          >
            创建 <span style={{ opacity: 0.6, marginLeft: 6, fontSize: 11 }}>⌘↵</span>
          </button>
        </div>
      </div>
    </div>
  );
}
