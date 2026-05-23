use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::store::with_conn;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub intent: String,
    pub profile: String,
    pub mode: String,
    pub state: String,
    pub workspace_path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Task {
    pub fn new(intent: String, profile: String, mode: String, workspace_path: String) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            intent,
            profile,
            mode,
            state: "Pending".into(),
            workspace_path,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn insert(&self) -> Result<()> {
        with_conn(|c| {
            c.execute(
                "INSERT INTO tasks (id, intent, profile, mode, state, workspace_path, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    self.id,
                    self.intent,
                    self.profile,
                    self.mode,
                    self.state,
                    self.workspace_path,
                    self.created_at.to_rfc3339(),
                    self.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_recent(limit: i64) -> Result<Vec<Task>> {
        with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, intent, profile, mode, state, workspace_path, created_at, updated_at
                 FROM tasks ORDER BY created_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit], |row| {
                Ok(Task {
                    id: row.get(0)?,
                    intent: row.get(1)?,
                    profile: row.get(2)?,
                    mode: row.get(3)?,
                    state: row.get(4)?,
                    workspace_path: row.get(5)?,
                    created_at: row
                        .get::<_, String>(6)?
                        .parse::<DateTime<Utc>>()
                        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e)))?,
                    updated_at: row
                        .get::<_, String>(7)?
                        .parse::<DateTime<Utc>>()
                        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e)))?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Enumerate every task in the database (no limit). Used at startup
    /// for orphan-task recovery — see `lib.rs::recover_orphan_tasks`.
    pub fn list_all() -> Result<Vec<Task>> {
        with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, intent, profile, mode, state, workspace_path, created_at, updated_at
                 FROM tasks ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(Task {
                    id: row.get(0)?,
                    intent: row.get(1)?,
                    profile: row.get(2)?,
                    mode: row.get(3)?,
                    state: row.get(4)?,
                    workspace_path: row.get(5)?,
                    created_at: row
                        .get::<_, String>(6)?
                        .parse::<DateTime<Utc>>()
                        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(e)))?,
                    updated_at: row
                        .get::<_, String>(7)?
                        .parse::<DateTime<Utc>>()
                        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e)))?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    pub fn get(id: &str) -> Result<Task> {
        with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, intent, profile, mode, state, workspace_path, created_at, updated_at
                 FROM tasks WHERE id = ?1",
            )?;
            let mut rows = stmt.query([id])?;
            let row = rows
                .next()?
                .ok_or_else(|| AppError::NotFound(format!("task {id}")))?;
            Ok(Task {
                id: row.get(0)?,
                intent: row.get(1)?,
                profile: row.get(2)?,
                mode: row.get(3)?,
                state: row.get(4)?,
                workspace_path: row.get(5)?,
                created_at: row
                    .get::<_, String>(6)?
                    .parse::<DateTime<Utc>>()
                    .map_err(|e| AppError::Other(e.to_string()))?,
                updated_at: row
                    .get::<_, String>(7)?
                    .parse::<DateTime<Utc>>()
                    .map_err(|e| AppError::Other(e.to_string()))?,
            })
        })
    }
}
