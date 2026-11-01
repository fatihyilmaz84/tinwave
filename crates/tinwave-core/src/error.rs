use thiserror::Error;

/// Errors produced by `tinwave-core` constructors and parsers.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoreError {
    /// The text is not a valid UUID.
    #[error("invalid id text {0:?}: not a UUID")]
    InvalidIdText(String),
    /// The UUID is valid but is not version 7, so it carries no creation time.
    #[error("id {0} is not a UUIDv7")]
    NotUuidV7(uuid::Uuid),
    /// The millisecond timestamp cannot be represented in a UUIDv7 (before 1970 or past 48 bits).
    #[error("timestamp {0} ms is outside the UUIDv7 range")]
    TimestampOutOfRange(i64),
    /// The random source failed, so no id could be generated.
    #[error("random source unavailable: {0}")]
    RandomUnavailable(String),
    /// A timezone offset outside the real-world range of -12:00 to +14:00.
    #[error("timezone offset {0} minutes is outside -720..=840")]
    TzOffsetOutOfRange(i16),
    /// The tier number is not 0 to 3.
    #[error("tier {0} does not exist (valid: 0 to 3)")]
    InvalidTier(u8),
}
