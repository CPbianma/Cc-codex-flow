import { useEffect, useState } from "react";
import { Task, api, ProbeResult } from "../lib/tauri-api";

interface Props {
  task: Task | null;
  probes: ProbeResult[];
  onRefreshProbes: () => void;
}

export function WorkspacePane({ task, probes, onRefreshProbes }: Props) {
  const [files, setFiles] = useState<string[]>([]);

  useEffect(() => {
    if (!task) {
      setFiles([]);
      return;
    }
    api.listWorkspaceFiles(task.id).then(setFiles).catch(() => setFiles([]));
  }, [task?.id]);

  return (
    <div className="pane pane-right">
      <div className="pane-header">Agents</div>
      <div className="pane-section">
        {probes.length === 0 && (
          <div className="dim" style={{ padding: "4px 10px" }}>
            正在探活…
          </div>
        )}
        {probes.map((p) => (
          <div key={p.agent} className="probe-row" title={p.binary_path ?? ""}>
            <span className={"dot " + (p.ok ? "ok" : "fail")} />
            <span className="agent-name">{p.agent}</span>
            <span
              className="dim"
              style={{
                marginLeft: "auto",
                maxWidth: 160,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {p.ok ? p.version ?? "?" : p.error?.slice(0, 40) ?? "n/a"}
            </span>
          </div>
        ))}
        <div className="probe-actions">
          <button className="btn-mini" onClick={onRefreshProbes}>
            重新探活
          </button>
        </div>
      </div>

      <div className="pane-header">Workspace</div>
      <div className="pane-body">
        {!task && <div className="empty">未选择任务</div>}
        {task && (
          <div className="workspace-path" title={task.workspace_path}>
            {task.workspace_path}
          </div>
        )}
        {task && files.length === 0 && (
          <div className="empty" style={{ padding: "12px 20px" }}>
            workspace 为空
          </div>
        )}
        {task && files.length > 0 && (
          <ul className="file-tree">
            {files.map((f) => (
              <li key={f}>{f}</li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
