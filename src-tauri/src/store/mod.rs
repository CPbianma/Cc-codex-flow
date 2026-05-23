use std::sync::Mutex;

use once_cell::sync::OnceCell;
use rusqlite::Connection;

use crate::error::Result;
use crate::paths::paths;

pub mod task;

static DB: OnceCell<Mutex<Connection>> = OnceCell::new();

const SCHEMA: &str = include_str!("schema.sql");

pub fn init() -> Result<()> {
    let conn = Connection::open(&paths().db_path)?;
    conn.execute_batch(SCHEMA)?;
    DB.set(Mutex::new(conn))
        .map_err(|_| crate::error::AppError::Other("DB already initialized".into()))?;
    Ok(())
}

pub fn with_conn<R>(f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
    let guard = DB
        .get()
        .ok_or_else(|| crate::error::AppError::Other("DB not initialized".into()))?
        .lock()
        .map_err(|e| crate::error::AppError::Other(format!("DB lock poisoned: {e}")))?;
    f(&guard)
}
