//! Idempotency keys for write requests (`Idempotency-Key` header).
//!
//! The caller chooses the key; this crate never generates one. The server
//! replays the first response for the same key and request, so a write that
//! timed out can be repeated safely. Reusing a key with a different request
//! is refused by the server (`IDEMPOTENCY_KEY_REUSED`).

use std::fmt;

/// Longest key the server accepts, in characters.
pub const MAX_LEN: usize = 255;

/// A validated idempotency key: 1 to [`MAX_LEN`] characters from
/// `A-Z a-z 0-9 . _ : -`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Validate `value`. The error does not repeat the value, so a mistyped
    /// secret never ends up in a message.
    pub fn parse(value: &str) -> Result<Self, InvalidIdempotencyKey> {
        let valid_len = (1..=MAX_LEN).contains(&value.len());
        let valid_chars = value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'));
        if valid_len && valid_chars {
            Ok(Self(value.to_string()))
        } else {
            Err(InvalidIdempotencyKey)
        }
    }

    /// The key as sent in the header.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The rejected key does not match the server's rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidIdempotencyKey;

impl fmt::Display for InvalidIdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "must be 1 to {MAX_LEN} characters from A-Z, a-z, 0-9, '.', '_', ':' or '-'"
        )
    }
}

impl std::error::Error for InvalidIdempotencyKey {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_documented_alphabet_and_lengths() {
        for valid in [
            "a",
            "order-2026.11_13:retry",
            "ABCxyz019",
            &"k".repeat(MAX_LEN),
        ] {
            let key = IdempotencyKey::parse(valid).unwrap_or_else(|_| panic!("{valid}"));
            assert_eq!(key.as_str(), valid);
        }
    }

    #[test]
    fn rejects_empty_long_and_foreign_characters() {
        for invalid in [
            "",
            &"k".repeat(MAX_LEN + 1),
            "has space",
            "slash/here",
            "uni\u{e9}",
            "new\nline",
            "semi;colon",
            "a=b",
        ] {
            assert_eq!(
                IdempotencyKey::parse(invalid),
                Err(InvalidIdempotencyKey),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn error_message_does_not_echo_the_value() {
        let message = InvalidIdempotencyKey.to_string();
        assert!(message.contains("1 to 255"), "{message}");
        assert!(!message.contains("secret"));
    }
}
