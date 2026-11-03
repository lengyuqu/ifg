//! `users` resource group (design doc §4.1). M1 covers the two calls the
//! `auth` commands need: the current profile and the token list.

use serde::Deserialize;

use crate::error::Error;
use crate::http::Client;

/// The `GET /users/me` profile. Only fields the CLI actually displays are
/// declared; unknown fields are ignored (D4), so server additions do not
/// break the client.
#[derive(Debug, Clone, Deserialize)]
pub struct UserProfile {
    pub id: i64,
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub is_admin: bool,
}

/// One entry of `GET /users/tokens` (a plain JSON array, not paginated).
#[derive(Debug, Clone, Deserialize)]
pub struct TokenInfo {
    pub id: i64,
    pub name: String,
    pub scopes: String,
}

/// Whether a stored scope list grants `required`, mirroring the server's
/// semantics (`admin` implies everything).
pub fn has_scope(granted: &str, required: &str) -> bool {
    granted
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .any(|scope| scope == "admin" || scope == required)
}

/// Accessor for user endpoints; borrows the client, holds no state.
pub struct Users<'a> {
    client: &'a Client,
}

impl Client {
    /// User endpoints (`/users/me`, `/users/tokens`, …).
    pub fn users(&self) -> Users<'_> {
        Users { client: self }
    }
}

impl Users<'_> {
    /// `GET /users/me` — requires the `user` PAT scope.
    /// `GET /users/{username}` — public profile (no email).
    pub async fn get(&self, username: &str) -> Result<UserProfile, Error> {
        self.client
            .send(crate::Method::GET, &format!("users/{username}"), None)
            .await
    }

    pub async fn me(&self) -> Result<UserProfile, Error> {
        self.client
            .send(reqwest::Method::GET, "users/me", None)
            .await
    }

    /// `GET /users/tokens` — the caller's PATs with their scopes.
    pub async fn tokens(&self) -> Result<Vec<TokenInfo>, Error> {
        self.client
            .send(reqwest::Method::GET, "users/tokens", None)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_scope_mirrors_server_semantics() {
        assert!(has_scope("repo,user", "repo"));
        assert!(has_scope("repo,user", "user"));
        assert!(!has_scope("repo", "user"));
        assert!(has_scope("admin", "repo"));
        assert!(!has_scope("", "repo"));
    }

    #[test]
    fn deserializes_me_response() {
        let json = r#"{"id":1,"username":"alice","email":"a@x","display_name":null,
                       "avatar_url":null,"is_admin":false,"created_at":"2026-01-01T00:00:00Z"}"#;
        let me: UserProfile = serde_json::from_str(json).unwrap();
        assert_eq!(me.username, "alice");
        assert_eq!(me.display_name, None);
    }
}
