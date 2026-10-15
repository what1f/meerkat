use std::fmt;

#[derive(Debug)]
pub enum StorageError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Invalid(String),
    UnsupportedSchemaVersion(i64),
    Conflict(String),
    LockPoisoned,
    Busy(String),
}
impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "storage_sqlite: {e}"),
            Self::Json(e) => write!(f, "storage_json: {e}"),
            Self::UnsupportedSchemaVersion(version) => write!(
                f,
                "unsupported storage schema version {version}; expected 1"
            ),
            Self::Invalid(e) => write!(f, "storage_invalid: {e}"),
            Self::Conflict(e) => write!(f, "storage_conflict: {e}"),
            Self::LockPoisoned => f.write_str("storage_lock_poisoned"),
            Self::Busy(e) => write!(f, "storage_busy: {e}"),
        }
    }
}
impl std::error::Error for StorageError {}
impl From<rusqlite::Error> for StorageError {
    fn from(v: rusqlite::Error) -> Self {
        Self::Sqlite(v)
    }
}
impl From<serde_json::Error> for StorageError {
    fn from(v: serde_json::Error) -> Self {
        Self::Json(v)
    }
}
pub type Result<T> = std::result::Result<T, StorageError>;

pub(super) fn conv(e: lankide_core::DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}
