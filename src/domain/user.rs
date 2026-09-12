use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::DomainError;

pub const MIN_PASSWORD_LEN: usize = 8;
pub const MAX_PASSWORD_LEN: usize = 72; // bcrypt operates on at most 72 bytes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
}

impl User {
    pub fn new(email: impl Into<String>, password_hash: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            email: email.into(),
            password_hash: password_hash.into(),
            created_at: Utc::now(),
        }
    }
}

/// Normalizes and validates an email: trimmed, lowercased, structurally sane.
pub fn normalize_email(raw: &str) -> Result<String, DomainError> {
    let email = raw.trim().to_lowercase();
    if email.len() < 3 || email.len() > 254 || !email.contains('@') || email.contains(' ') {
        return Err(DomainError::InvalidValue {
            field: "email".into(),
            reason: "must be a valid email address".into(),
        });
    }
    Ok(email)
}

/// Enforces password policy before hashing (bcrypt rejects > 72 bytes).
pub fn validate_password(raw: &str) -> Result<(), DomainError> {
    let len = raw.as_bytes().len();
    if len < MIN_PASSWORD_LEN {
        return Err(DomainError::InvalidValue {
            field: "password".into(),
            reason: format!("must be at least {MIN_PASSWORD_LEN} characters"),
        });
    }
    if len > MAX_PASSWORD_LEN {
        return Err(DomainError::InvalidValue {
            field: "password".into(),
            reason: format!("must be at most {MAX_PASSWORD_LEN} characters"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_normalization() {
        assert_eq!(normalize_email("  Alice@Example.COM ").unwrap(), "alice@example.com");
        assert!(normalize_email("nope").is_err());
        assert!(normalize_email("a b@c.com").is_err());
    }

    #[test]
    fn password_policy() {
        assert!(validate_password("longenough").is_ok());
        assert!(validate_password("short").is_err());
        assert!(validate_password(&"x".repeat(73)).is_err());
    }
}
