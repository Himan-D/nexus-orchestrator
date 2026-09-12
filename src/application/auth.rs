use std::sync::Arc;
use std::time::Duration;

use bcrypt::{hash, verify, DEFAULT_COST};
use chrono::{Duration as ChronoDuration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::user::{normalize_email, validate_password, User};
use crate::application::ports::{RepositoryError, UserRepository};

const TOKEN_TTL_HOURS: i64 = 24;
const BCRYPT_BLOCK: &str = "bcrypt hashing is CPU-bound";

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("an account with this email already exists")]
    EmailExists,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("invalid or expired token")]
    TokenInvalid,
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl From<RepositoryError> for AuthError {
    fn from(e: RepositoryError) -> Self {
        match e {
            RepositoryError::Conflict(_) => AuthError::EmailExists,
            other => AuthError::Internal(anyhow::anyhow!(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Debug, Clone)]
pub struct AuthedUser {
    pub id: Uuid,
    pub email: String,
}

pub struct AuthService {
    users: Arc<dyn UserRepository>,
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl AuthService {
    pub fn new(users: Arc<dyn UserRepository>, jwt_secret: &[u8]) -> Self {
        Self {
            users,
            encoding: EncodingKey::from_secret(jwt_secret),
            decoding: DecodingKey::from_secret(jwt_secret),
        }
    }

    pub async fn signup(&self, email: &str, password: &str) -> Result<(AuthedUser, String), AuthError> {
        let email = normalize_email(email)
            .map_err(|e| AuthError::InvalidInput(e.to_string()))?;
        validate_password(password)
            .map_err(|e| AuthError::InvalidInput(e.to_string()))?;

        // bcrypt is deliberately slow; keep it off the async runtime threads.
        let pw = password.to_string();
        let password_hash = tokio::task::spawn_blocking(move || {
            hash(pw, DEFAULT_COST).map_err(|e| AuthError::Internal(anyhow::anyhow!(e.to_string())))
        })
        .await
        .expect(BCRYPT_BLOCK)?;

        let user = User::new(email, password_hash);
        self.users.create(&user).await?;

        let authed = AuthedUser { id: user.id, email: user.email };
        let token = self.issue_token(&authed)?;
        Ok((authed, token))
    }

    pub async fn signin(&self, email: &str, password: &str) -> Result<(AuthedUser, String), AuthError> {
        let email = normalize_email(email)
            .map_err(|_| AuthError::InvalidCredentials)?;

        let user = self
            .users
            .get_by_email(&email)
            .await?
            .ok_or(AuthError::InvalidCredentials)?;

        let pw = password.to_string();
        let stored = user.password_hash.clone();
        let ok = tokio::task::spawn_blocking(move || {
            verify(&pw, &stored).unwrap_or(false)
        })
        .await
        .expect(BCRYPT_BLOCK);

        // Constant-ish response: same error for unknown user and bad password.
        if !ok {
            return Err(AuthError::InvalidCredentials);
        }

        let authed = AuthedUser { id: user.id, email: user.email };
        let token = self.issue_token(&authed)?;
        Ok((authed, token))
    }

    pub fn verify_token(&self, token: &str) -> Result<AuthedUser, AuthError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.leeway = 30;
        validation.validate_exp = true;

        let data = decode::<Claims>(token, &self.decoding, &validation)
            .map_err(|_| AuthError::TokenInvalid)?;

        let id = Uuid::parse_str(&data.claims.sub).map_err(|_| AuthError::TokenInvalid)?;
        Ok(AuthedUser { id, email: data.claims.email })
    }

    pub async fn me(&self, id: Uuid) -> Result<AuthedUser, AuthError> {
        let user = self
            .users
            .get(id)
            .await?
            .ok_or(AuthError::TokenInvalid)?;
        Ok(AuthedUser { id: user.id, email: user.email })
    }

    fn issue_token(&self, user: &AuthedUser) -> Result<String, AuthError> {
        let now = Utc::now();
        let claims = Claims {
            sub: user.id.to_string(),
            email: user.email.clone(),
            iat: now.timestamp(),
            exp: (now + ChronoDuration::hours(TOKEN_TTL_HOURS)).timestamp(),
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .map_err(|e| AuthError::Internal(anyhow::anyhow!(e.to_string())))
    }

    pub fn cookie_name() -> &'static str {
        "nexus_session"
    }

    pub fn session_cookie(&self, token: &str, secure: bool) -> String {
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={};{}",
            Self::cookie_name(),
            token,
            Duration::from_secs(TOKEN_TTL_HOURS as u64 * 3600).as_secs(),
            if secure { " Secure;" } else { "" }
        )
    }

    pub fn cleared_cookie(secure: bool) -> String {
        format!(
            "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0;{}",
            Self::cookie_name(),
            if secure { " Secure;" } else { "" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::sqlite::SqliteRepo;

    async fn test_service() -> Arc<AuthService> {
        let repo = SqliteRepo::in_memory().await.unwrap();
        repo.run_migrations().await.unwrap();
        Arc::new(AuthService::new(
            Arc::new(repo) as Arc<dyn UserRepository>,
            b"test-secret-test-secret-test-secret!",
        ))
    }

    #[tokio::test]
    async fn signup_signin_roundtrip() {
        let svc = test_service().await;
        let (u, token) = svc.signup("Alice@Example.com ", "hunter2222").await.unwrap();
        assert_eq!(u.email, "alice@example.com");

        let verified = svc.verify_token(&token).unwrap();
        assert_eq!(verified.id, u.id);

        let (_, token2) = svc.signin("alice@example.com", "hunter2222").await.unwrap();
        assert_eq!(svc.verify_token(&token2).unwrap().id, u.id);
    }

    #[tokio::test]
    async fn duplicate_signup_conflicts() {
        let svc = test_service().await;
        svc.signup("a@b.com", "hunter2222").await.unwrap();
        assert!(matches!(
            svc.signup("a@b.com", "hunter2222").await,
            Err(AuthError::EmailExists)
        ));
    }

    #[tokio::test]
    async fn wrong_password_rejected() {
        let svc = test_service().await;
        svc.signup("a@b.com", "hunter2222").await.unwrap();
        assert!(matches!(
            svc.signin("a@b.com", "wrongpass1").await,
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn weak_inputs_rejected() {
        let svc = test_service().await;
        assert!(matches!(
            svc.signup("a@b.com", "short").await,
            Err(AuthError::InvalidInput(_))
        ));
        assert!(matches!(
            svc.signup("not-an-email", "hunter2222").await,
            Err(AuthError::InvalidInput(_))
        ));
    }

    #[tokio::test]
    async fn tampered_token_rejected() {
        let svc = test_service().await;
        let (_, token) = svc.signup("a@b.com", "hunter2222").await.unwrap();
        let mut tampered = token.clone();
        tampered.replace_range(0..1, "x");
        assert!(matches!(
            svc.verify_token(&tampered),
            Err(AuthError::TokenInvalid)
        ));
    }
}
