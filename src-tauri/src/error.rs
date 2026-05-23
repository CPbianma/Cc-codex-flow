use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("serde_json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml ser: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("toml de: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("adapter: {0}")]
    Adapter(String),

    #[error("{0}")]
    Other(String),
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
