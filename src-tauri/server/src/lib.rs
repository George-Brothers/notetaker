//! Serving the Notetaker UI over HTTP, so the library can be read from a phone
//! or any browser on the same machine or network.
//!
//! This is what "web" means for this project, and it is worth being precise
//! about why it is not a web *app*. A browser cannot do the thing that makes
//! Notetaker good: it has no access to a 1–3 GB local speech model, no speaker
//! diarization, no Ollama, and no `~/Notetaker/Tasks` directory. System audio
//! capture in a browser is Chromium-only and returns silence in Safari and
//! Firefox.
//!
//! So the machine keeps doing all of it — capture, transcription, diarization,
//! summaries — and this serves the same React UI over HTTP, talking to the same
//! [`notetaker_core::dispatch`] entry point the desktop shell uses. The UI
//! cannot tell which transport it is on. Nothing leaves the machine.
//!
//! # Security posture
//!
//! **Loopback by default, and LAN access is an explicit opt-in that mints a
//! token.** A notetaker that quietly serves your meeting transcripts to the
//! coffee-shop wifi is a worse failure than any bug in it, so the safe mode is
//! the default and the unsafe one has to be asked for by name.
//!
//! On loopback there is no token: anything that can reach `127.0.0.1` is already
//! running as the user, and a token would only be security theatre. On a LAN
//! bind a token is required on every request, including the static assets —
//! serving the UI shell to a stranger tells them the machine is here.

pub mod auth;
pub mod router;
pub mod statics;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use notetaker_core::runtime::Runtime;

pub use auth::Token;
pub use router::{handle, Reply, Request};

/// How the server may be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// `127.0.0.1` only. No token — anything that can reach loopback is already
    /// this user.
    Loopback,
    /// All interfaces, so a phone on the same wifi can reach it. Requires a
    /// token on every request.
    Lan,
}

impl Access {
    /// The address to bind for this access mode.
    pub fn bind_ip(self) -> IpAddr {
        match self {
            Access::Loopback => IpAddr::V4(Ipv4Addr::LOCALHOST),
            Access::Lan => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        }
    }

    /// Whether requests must carry a token.
    pub fn requires_token(self) -> bool {
        matches!(self, Access::Lan)
    }
}

/// Everything the request handler needs.
pub struct Config {
    pub access: Access,
    pub port: u16,
    /// Directory holding the built UI (`dist/`). `None` serves the API only,
    /// which is what the integration tests use.
    pub ui_dir: Option<std::path::PathBuf>,
    /// Required on every request when [`Access::Lan`]; ignored on loopback.
    pub token: Option<Token>,
}

impl Config {
    /// The safe default: loopback, no token, on a port chosen to be memorable
    /// and out of the way of the Vite dev server.
    pub fn loopback(port: u16) -> Self {
        Self {
            access: Access::Loopback,
            port,
            ui_dir: None,
            token: None,
        }
    }

    /// LAN access with a freshly minted token.
    pub fn lan(port: u16) -> Self {
        Self {
            access: Access::Lan,
            port,
            ui_dir: None,
            token: Some(Token::generate()),
        }
    }

    pub fn with_ui_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.ui_dir = Some(dir.into());
        self
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.access.bind_ip(), self.port)
    }
}

/// Binds and serves until the process ends.
///
/// Blocking, one thread per request. `Runtime` is `Send + Sync` and internally
/// locked, so concurrent requests are safe; this is one person's own machine,
/// not a service.
pub fn serve(config: Config, runtime: Arc<Runtime>) -> Result<()> {
    let addr = config.addr();
    let server = tiny_http::Server::http(addr)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("could not start the Notetaker server on {addr}"))?;

    match config.access {
        Access::Loopback => {
            log::info!(
                "Notetaker UI on http://127.0.0.1:{} (this computer only)",
                config.port
            );
        }
        Access::Lan => {
            let token = config
                .token
                .as_ref()
                .map(|t| t.as_str())
                .unwrap_or("<none — refusing every request>");
            log::warn!(
                "Notetaker UI on port {} and reachable from your network. \
                 Open http://<this-computer>:{}/?token={}",
                config.port,
                config.port,
                token
            );
        }
    }

    for request in server.incoming_requests() {
        if let Err(e) = router::respond(&config, &runtime, request) {
            log::warn!("request failed: {e:#}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_binds_only_localhost() {
        assert_eq!(
            Access::Loopback.bind_ip(),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
        );
    }

    #[test]
    fn lan_binds_all_interfaces() {
        assert_eq!(Access::Lan.bind_ip(), IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)));
    }

    /// The security default. If this ever flips, meeting transcripts become
    /// reachable from whatever network the machine is on.
    #[test]
    fn the_default_configuration_is_loopback_with_no_token() {
        let c = Config::loopback(4321);
        assert_eq!(c.access, Access::Loopback);
        assert!(!c.access.requires_token());
        assert!(c.token.is_none());
        assert_eq!(c.addr().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    /// LAN mode must never be constructible without a token.
    #[test]
    fn lan_always_comes_with_a_token() {
        let c = Config::lan(4321);
        assert!(c.access.requires_token());
        assert!(c.token.is_some(), "LAN access without a token");
        assert!(c.token.unwrap().as_str().len() >= 32);
    }

    #[test]
    fn only_lan_requires_a_token() {
        assert!(!Access::Loopback.requires_token());
        assert!(Access::Lan.requires_token());
    }

    /// Two LAN configs must not share a token — a predictable token is no
    /// token at all.
    #[test]
    fn each_lan_config_gets_a_distinct_token() {
        let a = Config::lan(1).token.unwrap();
        let b = Config::lan(2).token.unwrap();
        assert_ne!(a.as_str(), b.as_str());
    }
}
