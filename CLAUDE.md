# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Flow is a lightweight multi-agent collaboration framework. The Tauri 2 + React 19 desktop shell drives the `claude` and `codex` CLIs as subprocesses through a 3-round finite-state machine (R1 → R2, with role-swap R3) per task. There are two built-in profiles: `dev` (code work) and `visual` (design/spec work). The Rust core in `src-tauri/` owns the orchestrator, adapters, and store; the React frontend in `src/` is a thin UI on top of 12 Tauri commands.

## Layout

- `src/` — React 19 frontend (Vite + TS). UI talks to backend via `@tauri-apps/api/core` invoke.
- `src-tauri/src/`
  - `commands.rs` — the 12 `#[tauri::command]` functions exposed to the frontend. **Don't change their signatures** without also updating the frontend callers.
  - `orchestrator/fsm.rs` — the round-by-round FSM that drives turns, writes streams, evaluates reviewer verdicts.
  - `adapter/{claude,codex,mod}.rs` — CLI adapters. On Windows they auto-wrap `.cmd`/`.bat` via `cmd.exe /C` and `.ps1` via `powershell.exe -File` (Windows refuses to spawn batch files directly with arbitrary args via CreateProcess).
  - `bridge/`, `profile/`, `store/`, `paths.rs`, `workspace.rs`, `settings.rs`, `error.rs`.
- `src-tauri/profiles/{dev,visual}.toml` + `templates/*.md` — the three roles per profile (decider / executor / reviewer) and their prompt templates.
- `src-tauri/examples/e2e_{dev,visual}.rs` — headless E2E drivers (orchestrator without GUI). Run via `cargo run --example`.

## Build / test commands

PowerShell on Windows; chain with `;` and `if ($?)`, **never** `&&`.

```powershell
pnpm dev                                         # Vite dev server (frontend only)
pnpm build                                       # tsc && vite build
pnpm tauri dev                                   # full app
cd src-tauri ; cargo build --color never         # Rust build
cd src-tauri ; cargo test --lib                  # 18+ unit tests
& 'node_modules\.bin\tsc.cmd' --noEmit           # TS typecheck without build
cd src-tauri ; cargo run --example e2e_dev       # headless dev E2E
cd src-tauri ; cargo run --example e2e_visual    # headless visual E2E
```

## FSM contract

States: `Pending → R{1,2,3}_{Deciding,Executing,Reviewing} → Done | Failed | NeedsHuman`.

- Reviewer writes `decisions/{n:03}-review.md` (e.g. `001-review.md`). The **first non-empty line** must be exactly `PASS` or `FAIL:<reason>` — the parser keys off that.
- Interventions are file-based, polled from `meta/`:
  - `meta/control.json` — pause/resume
  - `meta/abort.flag` — hard abort
  - `meta/retry.flag` — re-run current turn
  - `meta/feedback.jsonl` — user-injected guidance
- Streaming output for each turn lands in `meta/turns/r{n}-{role}.stream.jsonl`.

## Profile template conventions

Templates use `{n}` (zero-padded round, e.g. `001`) in filename references — **not** `{round}`. The FSM expects the padded form when looking up reviewer files like `decisions/001-review.md`. Round 2 of the role-swap visual flow writes to `002-...`, etc.

## Style preservation (non-negotiable)

- Anthropic light palette must stay: `--cream`, `--clay`, Source Serif 4 font, hex tokens like `#FAF6EE`. Don't switch to dark or generic neutral grays.
- Chinese-language strings in the UI must be preserved (this is a CN-locale build).
- No native-binding npm packages (`sharp`, `canvas`, `node-gyp`, etc.) — they break the pnpm install on Windows. `.npmrc` enforces `verify-deps-before-run=false` and `ignored-built-dependencies=esbuild`.

## Tauri command stability

The 12 commands in `src-tauri/src/commands.rs` form the IPC contract:
`create_task`, `list_tasks`, `get_task`, `list_workspace_files`, `read_workspace_file`, `probe_agents`, `get_settings`, `set_workspaces_root`, `reset_task`, `start_task`, `get_task_state`, `intervene`. Renaming or changing their parameter shapes breaks the frontend — always update both sides together.

## Local CLI paths

- `claude` (Claude Code 2.1.x) — `C:\Users\<user>\.local\bin\claude.cmd`
- `codex` (codex-cli 0.133.x) — `E:\develop\nodejs\codex.cmd`

Override with env vars `FLOW_CLAUDE_BIN` and `FLOW_CODEX_BIN` when running E2E examples. Headless runs use `Permission::FullAuto`, which maps to `--permission-mode bypassPermissions` (Claude) and `-s danger-full-access` (Codex) so subprocesses never block on interactive prompts.

## Windows shell notes

- `;` to chain (PowerShell 5.1 has no `&&`); `if ($?) { ... }` for conditional chaining.
- `$env:VAR = 'x'` to set env vars, not `export`.
- Invoke executables with spaces in path via `& 'C:\path with space\app.exe' args`.
- Use UTF-8 explicitly when writing files other tools will read: `Out-File -Encoding utf8`.
