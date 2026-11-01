use thiserror::Error;

use crate::SignalKind;

/// Errors from the signal schema and the packed payload codec.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SignalsError {
    /// The stored `kind` column holds a value that is not a known discriminant.
    #[error("unknown signal kind discriminant {0}")]
    UnknownKind(u8),
    /// The kind is a counter or derived signal; it never has a packed event payload.
    #[error("signal kind {0} is not stored as a raw event and has no packed payload")]
    NotAnEvent(SignalKind),
    /// The kind requires payload fields but the stored payload is `NULL`.
    #[error("payload for {0} is missing but the kind carries fields")]
    MissingPayload(SignalKind),
    /// The kind has no payload fields but a payload was stored.
    #[error("payload for {0} must be empty (NULL)")]
    UnexpectedPayload(SignalKind),
    /// The payload was written with a version this build cannot decode.
    #[error("payload version {version} of {kind} is not supported")]
    UnsupportedVersion {
        /// Kind being decoded.
        kind: SignalKind,
        /// Version byte found in the payload.
        version: u8,
    },
    /// The payload ended before all fields were read.
    #[error("payload is truncated")]
    Truncated,
    /// Bytes remained after the last field.
    #[error("{0} trailing byte(s) after the payload")]
    TrailingBytes(usize),
    /// A variable-length integer is longer than 32 bits.
    #[error("variable-length integer overflows 32 bits")]
    VarintOverflow,
    /// A variable-length integer was not in its shortest form.
    #[error("variable-length integer is not in canonical (shortest) form")]
    NonCanonicalVarint,
    /// A field holds a value outside its domain.
    #[error("value {value} is not a valid {what}")]
    InvalidValue {
        /// Name of the field type.
        what: &'static str,
        /// The offending raw value.
        value: u32,
    },
    /// A floating point input was NaN or infinite.
    #[error("{what} must be a finite number")]
    NotFinite {
        /// Name of the quantity.
        what: &'static str,
    },
    /// The encoded payload would exceed the 12-byte budget.
    #[error("encoded payload exceeds {} bytes", crate::MAX_PAYLOAD_LEN)]
    PayloadTooLong,
}
