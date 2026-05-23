import { Task } from "../lib/tauri-api";

interface Props {
  tasks: Task[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onDelete: (task: Task) => void;
}

export function TaskList({ tasks, selectedId, onSelect, onNew, onDelete }: Props) {
  return (
    <div className="pane pane-left">
      <div className="pane-header">
        <span>任务</span>
        <button className="btn-mini primary" onClick={onNew}>
          新建任务
        </button>
      </div>
      <div className="pane-body">
        {tasks.length === 0 && (
          <div className="empty">
            还没有任务
            <div style={{ marginTop: 8, fontSize: 11, color: "var(--ink-faint)" }}>
              点击右上角「新建」开始
            </div>
          </div>
        )}
        {tasks.length > 0 && (
          <div className="task-list">
            {tasks.map((t) => (
              <div
                key={t.id}
                className={"task-row" + (t.id === selectedId ? " selected" : "")}
                onClick={() => onSelect(t.id)}
              >
                <div className="task-intent" title={t.intent}>
                  {t.intent}
                </div>
                <div className="task-meta">
                  <span className={"badge badge-" + t.profile}>{t.profile}</span>
                  <span className="badge">{t.mode}</span>
                  <span className="badge state">{t.state}</span>
                </div>
                <button
                  type="button"
                  className="task-delete"
                  title="删除任务（产物归档到 _archive/）"
                  aria-label="删除任务"
                  onClick={(e) => {
                    e.stopPropagation();
                    onDelete(t);
                  }}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
