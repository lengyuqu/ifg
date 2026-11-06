//! Collaboration requests use the user's PAT and, for agent actions, a
//! separate session credential. Credentials are never included in URLs.
use serde_json::{json, Value};

use crate::{Client, Error, Method, Secret};

#[derive(Clone)]
pub struct SessionCredentials {
    pub id: String,
    pub(crate) secret: Secret,
}

impl std::fmt::Debug for SessionCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCredentials")
            .field("id", &self.id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl SessionCredentials {
    pub fn new(id: impl Into<String>, secret: Secret) -> Self {
        Self {
            id: id.into(),
            secret,
        }
    }
    pub fn secret(&self) -> &Secret {
        &self.secret
    }
}

#[derive(Debug)]
pub struct StartedSession {
    pub session: Value,
    pub credentials: SessionCredentials,
}

pub struct Collaboration<'a> {
    client: &'a Client,
    prefix: String,
}

impl Client {
    pub fn collaboration(&self, owner: &str, repo: &str) -> Collaboration<'_> {
        Collaboration {
            client: self,
            prefix: format!("repos/{}/{}/collaboration", segment(owner), segment(repo)),
        }
    }
}

/// Encode untrusted path segments without allowing them to change the route.
pub fn segment(value: &str) -> String {
    // URL parsers normalize dot segments even when percent encoded once.
    if value == "." || value == ".." {
        return value.replace('.', "%252E");
    }
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn redact(value: &mut Value, secret: &str) {
    if secret.is_empty() {
        return;
    }
    match value {
        Value::String(text) => *text = text.replace(secret, "[REDACTED]"),
        Value::Array(items) => items.iter_mut().for_each(|item| redact(item, secret)),
        Value::Object(map) => {
            for (key, mut item) in std::mem::take(map) {
                redact(&mut item, secret);
                map.insert(key.replace(secret, "[REDACTED]"), item);
            }
        }
        _ => {}
    }
}

impl Collaboration<'_> {
    pub async fn start_session(
        &self,
        display_name: &str,
        client_kind: &str,
    ) -> Result<StartedSession, Error> {
        let body = json!({"display_name": display_name, "client_kind": client_kind});
        let bytes = self
            .client
            .collab_raw(
                Method::POST,
                &format!("{}/sessions", self.prefix),
                Some(&body),
                None,
            )
            .await?;
        // Do not include the response body in decode errors: it contains a secret.
        let mut response: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Config("invalid session creation response".into()))?;
        let secret = response
            .get_mut("session_secret")
            .map(Value::take)
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or_else(|| Error::Config("session response lacks a secret".into()))?;
        let mut session = response
            .get("session")
            .cloned()
            .ok_or_else(|| Error::Config("session response lacks session".into()))?;
        redact(&mut session, &secret);
        let id = session["id"]
            .as_str()
            .ok_or_else(|| Error::Config("session response lacks id".into()))?
            .to_string();
        Ok(StartedSession {
            session,
            credentials: SessionCredentials::new(id, Secret::new(secret)),
        })
    }

    /// Execute an operation relative to this repository's collaboration root.
    /// Callers must encode individual path segments and query values.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        session: Option<&SessionCredentials>,
    ) -> Result<Value, Error> {
        let bytes = self
            .client
            .collab_raw(
                method,
                &format!("{}/{}", self.prefix, path.trim_start_matches('/')),
                body,
                session,
            )
            .await?;
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        let mut value: Value = serde_json::from_slice(&bytes).map_err(|source| Error::Decode {
            body_snippet: "collaboration response".into(),
            source,
        })?;
        if let Some(session) = session {
            redact(&mut value, session.secret.expose());
        }
        Ok(value)
    }
}
