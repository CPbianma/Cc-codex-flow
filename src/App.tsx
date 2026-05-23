import { useCallback, useEffect, useMemo, useState } from "react";
import { TaskList } from "./components/TaskList";
import { ThreadView } from "./components/ThreadView";
import { WorkspacePane } from "./components/WorkspacePane";
import { NewTaskDialog } from "./components/NewTaskDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { ProbeResult, Task, api } from "./lib/tauri-api";
import "./App.css";

export default function App() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [probes, setProbes] = useState<ProbeResult[]>([]);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await api.listTasks();
      setTasks(list);
      if (!selectedId && list[0]) setSelectedId(list[0].id);
    } catch (e: any) {
      setError(String(e));
    }
  }, [selectedId]);

  const refreshProbes = useCallback(async () => {
    try {
      setProbes(await api.probeAgents());
    } catch (e: any) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
    refreshProbes();
  }, []);

  const selectedTask = useMemo(
    () => tasks.find((t) => t.id === selectedId) ?? null,
    [tasks, selectedId]
  );

  const handleCreate = async (input: {
    intent: string;
    profile: string;
    mode: string;
  }) => {
    try {
      const out = await api.createTask(input);
      setDialogOpen(false);
      setSelectedId(out.task.id);
      await refresh();
    } catch (e: any) {
      setError(String(e));
    }
  };

  const handleDelete = async (task: Task) => {
    const ok = window.confirm(
      `确定删除「${task.intent}」？\n\n` +
        "• 任务从列表移除\n" +
        "• 产物归档到 _archive/<时间戳>-<前缀>/（保留 intent / decisions / execution / artifacts）\n" +
        "• 如果正在运行，会强制中止 claude/codex 子进程\n\n" +
        "此操作不可撤销。"
    );
    if (!ok) return;
    try {
      await api.deleteTask(task.id);
      if (selectedId === task.id) setSelectedId(null);
      await refresh();
    } catch (e: any) {
      setError(String(e));
    }
  };

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden />
          <span>Flow</span>
        </div>
        <div className="brand-sub">Claude × Codex orchestrator</div>
        {error && (
          <div className="err" onClick={() => setError(null)}>
            {error}
          </div>
        )}
        <button
          className="btn-mini"
          onClick={() => setSettingsOpen(true)}
          style={{ marginLeft: error ? 12 : "auto" }}
          title="设置"
        >
          设置
        </button>
      </header>
      <div className="cols">
        <TaskList
          tasks={tasks}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onNew={() => setDialogOpen(true)}
          onDelete={handleDelete}
        />
        <ThreadView task={selectedTask} onNewTask={() => setDialogOpen(true)} />
        <WorkspacePane
          task={selectedTask}
          probes={probes}
          onRefreshProbes={refreshProbes}
        />
      </div>
      <NewTaskDialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
        onSubmit={handleCreate}
      />
      <SettingsDialog
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
      />
    </div>
  );
}
