//! The token that guards LAN access.

use std::fmt;

/// A bearer token for LAN access.
///
/// Debug-prints as `Token(<redacted>)`. The whole point of a token is that it
/// does not appear anywhere it can be read later, and a log line or a panic
/// message that helpfully includes it defeats it entirely.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// A fresh random token.
    ///
    /// Two v4 UUIDs with the hyphens removed: 256 bits of randomness, and
    /// URL-safe so it can be pasted into a phone's address bar as `?token=...`.
    pub fn generate() -> Self {
        let a = uuid::Uuid::new_v4().simple().to_string();
        let b = uuid::Uuid::new_v4().simple().to_string();
        Token(format!("{a}{b}"))
    }

    pub fn from_string(s: impl Into<String>) -> Self {
        Token(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant-time comparison.
    ///
    /// A short-circuiting `==` leaks how many leading characters of a guess were
    /// right, which over enough requests recovers the token one character at a
    /// time. That attack is not realistic over a home wifi against a token this
    /// long, but the correct comparison costs one line.
    pub fn matches(&self, candidate: &str) -> bool {
        let expected = self.0.as_bytes();
        let got = candidate.as_bytes();
        // Compare the lengths without returning early, then every byte.
        let mut diff = (expected.len() ^ got.len()) as u8;
        for i in 0..expected.len().max(got.len()) {
            let e = expected.get(i).copied().unwrap_or(0);
            let g = got.get(i).copied().unwrap_or(0);
            diff |= e ^ g;
        }
        diff == 0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_token_is_long_and_url_safe() {
        let t = Token::generate();
        assert_eq!(t.as_str().len(), 64);
        assert!(
            t.as_str().chars().all(|c| c.is_ascii_alphanumeric()),
            "token must be safe to paste into a URL: {}",
            t.as_str()
        );
    }

    #[test]
    fn tokens_are_not_repeated() {
        assert_ne!(Token::generate().as_str(), Token::generate().as_str());
    }

    #[test]
    fn a_token_matches_itself() {
        let t = Token::generate();
        let copy = t.as_str().to_string();
        assert!(t.matches(&copy));
    }

    #[test]
    fn a_wrong_token_does_not_match() {
        let t = Token::from_string("abc123");
        assert!(!t.matches("abc124"));
        assert!(!t.matches(""));
        assert!(!t.matches("abc1234"), "a longer guess must not match");
        assert!(!t.matches("abc12"), "a prefix must not match");
    }

    /// The reason this test exists: a `Debug` derive would print the token into
    /// any log line or panic message that formats the config.
    #[test]
    fn debug_output_never_contains_the_token() {
        let t = Token::from_string("supersecrettokenvalue");
        let shown = format!("{t:?}");
        assert!(
            !shown.contains("supersecret"),
            "the token leaked into Debug output: {shown}"
        );
        assert_eq!(shown, "Token(<redacted>)");
    }
}
