# Flow

让 Claude Code 和 Codex CLI 一起把活干完 —— 一个桌面应用、3 轮 FSM、全程可观察、可暂停可中止可改主意。

Flow 是一个轻量的多 agent 协作框架，把 `claude` 和 `codex` 两个 CLI 当子进程编排起来：**决策者**出方案、**执行者**写代码、**审查者**把关 —— PASS 一次就结束，FAIL 进下一轮，最多 3 轮（R3 角色互换）。所有产物落在独立 workspace 里，UI 可暂停 / 继续 / 重试 / 中止 / 给下一轮反馈 / 删除（带归档）。

## 截图

| 欢迎页 | 新建任务 |
|---|---|
| ![welcome](docs/screenshots/01-welcome.png) | ![new task](docs/screenshots/02-new-task-dialog.png) |

| 任务运行中 | 干预按钮 |
|---|---|
| ![running](docs/screenshots/03-running-task.png) | ![intervene](docs/screenshots/04-intervene-buttons.png) |

## 它做什么

- **3 轮 FSM** — `Pending → R1{Decide→Execute→Review} → R2 → R3(角色互换) → Done | Failed | NeedsHuman`
- **两个内置 profile** ：
  - `dev` — Claude 决策 + 审查，Codex 执行。写代码用
  - `visual` — 先出 wireframe + visual spec，再实现。视觉敏感的活用
- **文件协议干预** — 暂停 / 中止 / 重试 / 反馈 全部通过 `meta/` 下的文件触发，UI 只是包装。无锁，幂等
- **删除带归档** — 从列表删任务时，把 `intent.md` / `decisions/` / `execution/` / `artifacts/` 归档到 `_archive/<时间戳>-<intent前缀>/`，剥掉 `meta/` 等中间物。永远可追溯

## 技术栈

- **前端** — Tauri 2 + React 19 + Vite + TypeScript
- **后端** — Rust（tokio + rusqlite + tauri 2.11）
- **依赖的 CLI** — [Claude Code CLI](https://docs.claude.com/en/docs/claude-code/overview) 2.1.x + [Codex CLI](https://github.com/openai/codex) 0.133.x
- **平台** — Windows 11（Tauri 适配器对 `.cmd` / `.ps1` 做了 `cmd.exe /C` / `powershell.exe -File` 包装）

## 快速上手

详细文档见 [`docs/USAGE.md`](docs/USAGE.md)。最小路径：

```powershell
# 1. 依赖（Node 22+ / pnpm 9+ / Rust stable / Claude Code CLI / Codex CLI 都装好）
pnpm install

# 2. 跑起来（自动起 Vite + Tauri 桌面壳）
pnpm tauri dev

# 3. 也可以无 GUI 端到端跑
cd src-tauri ; cargo run --example e2e_dev
```

CLI 不在 PATH 里时设环境变量覆盖：

```powershell
$env:FLOW_CLAUDE_BIN = 'C:\Users\<user>\.local\bin\claude.cmd'
$env:FLOW_CODEX_BIN  = 'E:\develop\nodejs\codex.cmd'
```

## 项目结构

```
src/                          React 前端，13 个 Tauri command 的薄包装
src-tauri/
  src/
    commands.rs                IPC 入口（13 个 #[tauri::command]）
    orchestrator/fsm.rs        FSM 主循环
    adapter/{claude,codex}.rs  CLI 适配器（Windows 上自动 cmd.exe /C 包装）
    bridge/{role,permission}.rs 角色模板渲染 + 权限映射
    store/                     SQLite 任务表
    workspace.rs               目录管理 + 归档
  profiles/                    dev / visual 两套 profile + 6 个角色模板
  examples/                    e2e_dev / e2e_visual 无 GUI 驱动
docs/                          USAGE 教程 + 截图 + 证据日志
```

## 干预协议（`meta/` 下的文件）

| 文件 | 作用 |
|---|---|
| `control.json` | `{"paused": true/false}` — FSM 在 spin-loop 里轮询 |
| `abort.flag` | 存在即中止；FSM 转到 Failed，删除标志 |
| `retry.flag` | 存在即把当前轮回退到 Deciding |
| `feedback.jsonl` | 追加用户反馈；下一轮 Decider 会读 |
| `state.json` | 当前 FSM 状态 + 历史，UI 每 1.5s 轮询 |
| `turns/r{n}-{role}.stream.jsonl` | 每个 turn 的流式输出 |

## License

MIT（如需 LICENSE 文件可后续补）
