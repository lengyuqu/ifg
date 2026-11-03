//! A personal access token that never prints in full.

use std::fmt;

/// Number of characters kept visible when a token is printed: the `ifp_`
/// prefix plus four characters, enough to tell tokens apart.
const VISIBLE_CHARS: usize = 8;

/// Wraps a PAT so `Debug` and `Display` show only `ifp_xxxx…`, keeping the
/// token out of logs and error messages. Use [`Secret::expose`] to get the
/// real value when building a request.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// The full token, for the `Authorization` header and nothing else.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The printable form: the first [`VISIBLE_CHARS`] characters followed by `…`.
    pub fn redacted(&self) -> String {
        let visible: String = self.0.chars().take(VISIBLE_CHARS).collect();
        format!("{visible}…")
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Secret").field(&self.redacted()).finish()
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.redacted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "ifp_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn display_and_debug_hide_the_token() {
        let secret = Secret::new(TOKEN);
        assert_eq!(secret.to_string(), "ifp_0123…");
        assert_eq!(format!("{secret:?}"), "Secret(\"ifp_0123…\")");
    }

    #[test]
    fn expose_returns_the_full_token() {
        assert_eq!(Secret::new(TOKEN).expose(), TOKEN);
    }

    #[test]
    fn short_tokens_do_not_panic() {
        assert_eq!(Secret::new("ab").to_string(), "ab…");
        assert_eq!(Secret::new("").to_string(), "…");
    }
}
