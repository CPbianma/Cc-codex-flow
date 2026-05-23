CREATE TABLE IF NOT EXISTS tasks (
    id              TEXT PRIMARY KEY,
    intent          TEXT NOT NULL,
    profile         TEXT NOT NULL,
    mode            TEXT NOT NULL DEFAULT 'auto',
    state           TEXT NOT NULL DEFAULT 'Pending',
    workspace_path  TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(created_at DESC);

CREATE TABLE IF NOT EXISTS turns (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    round           INTEGER NOT NULL,
    role            TEXT NOT NULL,        -- decider | executor | reviewer
    agent           TEXT NOT NULL,        -- claude | codex
    state           TEXT NOT NULL,        -- pending | running | done | failed
    started_at      TEXT,
    finished_at     TEXT,
    duration_ms     INTEGER,
    exit_code       INTEGER,
    artifacts_json  TEXT,                 -- JSON array of relative paths
    raw_log_path    TEXT,
    created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_turns_task_id ON turns(task_id, round);
