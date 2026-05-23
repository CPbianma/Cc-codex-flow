import { invoke } from "@tauri-apps/api/core";

export type AgentId = "claude" | "codex";

export interface Task {
  id: string;
  intent: string;
  profile: string;
  mode: string;
  state: string;
  workspace_path: string;
  created_at: string;
  updated_at: string;
}

export interface CreateTaskInput {
  intent: string;
  profile?: string;
  mode?: string;
}

export interface CreateTaskOutput {
  task: Task;
  workspace_path: string;
}

export interface ProbeResult {
  agent: AgentId;
  binary_path: string | null;
  version: string | null;
  ok: boolean;
  error: string | null;
}

export interface Settings {
  default_profile: string;
  default_mode: string;
  workspaces_root: string | null;
  claude_cli_path: string | null;
  codex_cli_path: string | null;
}

export const api = {
  createTask: (input: CreateTaskInput) =>
    invoke<CreateTaskOutput>("create_task", { input }),
  listTasks: (limit?: number) =>
    invoke<Task[]>("list_tasks", { limit: limit ?? null }),
  getTask: (id: string) => invoke<Task>("get_task", { id }),
  listWorkspaceFiles: (id: string) =>
    invoke<string[]>("list_workspace_files", { id }),
  readWorkspaceFile: (id: string, relativePath: string) =>
    invoke<string>("read_workspace_file", { id, relativePath }),
  probeAgents: () => invoke<ProbeResult[]>("probe_agents"),
  getSettings: () => invoke<Settings>("get_settings"),
  setWorkspacesRoot: (path: string) =>
    invoke<Settings>("set_workspaces_root", { path }),
  startTask: (id: string) => invoke<void>("start_task", { id }),
  getTaskState: (id: string) => invoke<string>("get_task_state", { id }),
  intervene: (id: string, action: string) =>
    invoke<void>("intervene", { id, action }),
  resetTask: (id: string) => invoke<void>("reset_task", { id }),
  deleteTask: (id: string) => invoke<void>("delete_task", { id }),
};
