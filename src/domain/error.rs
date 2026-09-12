use thiserror::Error;

/// Errors produced by the domain layer (pure business rules).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("Invalid state transition from {from} to {to}")]
    InvalidStateTransition { from: String, to: String },

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid value for {field}: {reason}")]
    InvalidValue { field: String, reason: String },
}
