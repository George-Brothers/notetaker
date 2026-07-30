//! Turning an HTTP request into a [`notetaker_core::dispatch`] call or a file.
//!
//! Split into a pure [`handle`] and a thin [`respond`] that does the I/O, for
//! the same reason as everywhere else in this project: the routing, the auth
//! decision and the error mapping are the parts that can be wrong, so they are
//! a function over values and are tested, while the socket handling is four
//! lines with nothing to decide.

use std::sync::Arc;

use anyhow::Result;
use notetaker_core::runtime::Runtime;
use serde_json::{json, Value};

use crate::{statics, Config};

/// A request, reduced to what routing actually needs.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// Full URL path, query string included.
    pub url: String,
    /// The `token` query parameter or `X-Notetaker-Token` header, if present.
    pub token: Option<String>,
    /// JSON body, for API calls.
    pub body: Option<Value>,
}

/// What to send back.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Json {
        status: u16,
        body: Value,
    },
    /// Serve this file from disk. The router resolves the path; `respond` reads
    /// it, so a router test never touches the filesystem.
    File {
        path: std::path::PathBuf,
    },
    /// The app shell, for a client-side route like `/settings`.
    AppShell,
}

/// The API path prefix. A command is `POST /api/<command_name>`.
const API_PREFIX: &str = "/api/";

/// Decides what a request should get. Pure.
pub fn handle(config: &Config, request: &Request) -> Reply {
    // Auth first, and before anything is revealed — including the UI shell.
    // Serving the app to an unauthenticated stranger tells them this machine is
    // here and running Notetaker.
    if config.access.requires_token() && !token_ok(config, request) {
        return Reply::Json {
            status: 401,
            body: json!({ "error": "This link needs the access code shown when you turned on network access." }),
        };
    }

    let path = request.url.split(['?', '#']).next().unwrap_or("");

    if let Some(command) = path.strip_prefix(API_PREFIX) {
        return route_api(request, command);
    }

    // Anything else is the UI.
    let Some(ui_dir) = config.ui_dir.as_ref() else {
        return Reply::Json {
            status: 404,
            body: json!({ "error": "This Notetaker server is running without its user interface." }),
        };
    };

    match statics::resolve(ui_dir, path) {
        Some(file) => Reply::File { path: file },
        // A refused path is a traversal attempt, not a missing page. Answering
        // 404 rather than 403 declines to confirm what is or is not there.
        None => Reply::Json {
            status: 404,
            body: json!({ "error": "Not found." }),
        },
    }
}

/// Routes an `/api/<command>` request.
fn route_api(request: &Request, command: &str) -> Reply {
    if request.method != "POST" {
        return Reply::Json {
            status: 405,
            body: json!({ "error": "Commands must be sent as POST." }),
        };
    }
    if !notetaker_core::dispatch::is_known_command(command) {
        return Reply::Json {
            status: 404,
            body: json!({ "error": format!("Unknown command {command:?}.") }),
        };
    }
    // A recognized command with a body is the only case the caller executes.
    Reply::Json {
        status: 0, // sentinel: "run the command"
        body: request.body.clone().unwrap_or_else(|| json!({})),
    }
}

/// Marker for "this reply means: execute the command".
///
/// A sentinel status rather than another enum variant, because it keeps [`Reply`]
/// describing only what goes on the wire. [`is_command`] is the only reader.
pub fn is_command(reply: &Reply) -> bool {
    matches!(reply, Reply::Json { status: 0, .. })
}

fn token_ok(config: &Config, request: &Request) -> bool {
    let Some(expected) = config.token.as_ref() else {
        // LAN access configured with no token: refuse everything rather than
        // allow everything. A misconfiguration must fail closed.
        return false;
    };
    match request.token.as_deref() {
        Some(given) => expected.matches(given),
        None => false,
    }
}

/// Extracts the `token` query parameter from a URL.
pub fn token_from_url(url: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == "token").then(|| v.to_string())
    })
}

/// Runs a command through core's dispatcher and shapes the reply.
///
/// Errors become a 400 carrying the runtime's own plain-English message. The
/// runtime already writes for a non-engineer, so nothing is rewritten here.
pub fn run_command(runtime: &Runtime, command: &str, args: &Value) -> Reply {
    match notetaker_core::dispatch::dispatch(runtime, command, args) {
        Ok(value) => Reply::Json {
            status: 200,
            body: value,
        },
        Err(e) => Reply::Json {
            status: 400,
            body: json!({ "error": format!("{e:#}") }),
        },
    }
}

/// Reads a `tiny_http` request, routes it, and writes the response.
pub fn respond(
    config: &Config,
    runtime: &Arc<Runtime>,
    mut request: tiny_http::Request,
) -> Result<()> {
    let url = request.url().to_string();
    let method = request.method().as_str().to_string();

    let header_token = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("X-Notetaker-Token"))
        .map(|h| h.value.as_str().to_string());
    let token = header_token.or_else(|| token_from_url(&url));

    let body = if method == "POST" {
        let mut raw = String::new();
        std::io::Read::read_to_string(request.as_reader(), &mut raw)?;
        serde_json::from_str(&raw).ok()
    } else {
        None
    };

    let parsed = Request {
        method,
        url: url.clone(),
        token,
        body,
    };

    let mut reply = handle(config, &parsed);
    if is_command(&reply) {
        let command = url
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .strip_prefix(API_PREFIX)
            .unwrap_or("")
            .to_string();
        let args = match &reply {
            Reply::Json { body, .. } => body.clone(),
            _ => json!({}),
        };
        reply = run_command(runtime, &command, &args);
    }

    match reply {
        Reply::Json { status, body } => {
            let data = serde_json::to_vec(&body)?;
            let header = "Content-Type: application/json; charset=utf-8"
                .parse::<tiny_http::Header>()
                .map_err(|_| anyhow::anyhow!("bad header"))?;
            request.respond(
                tiny_http::Response::from_data(data)
                    .with_status_code(status)
                    .with_header(header),
            )?;
        }
        Reply::File { path } => match std::fs::read(&path) {
            Ok(data) => {
                let header = format!("Content-Type: {}", statics::content_type(&path))
                    .parse::<tiny_http::Header>()
                    .map_err(|_| anyhow::anyhow!("bad header"))?;
                request.respond(tiny_http::Response::from_data(data).with_header(header))?;
            }
            // A missing asset falls through to the app shell so client-side
            // routes like /settings work on a hard refresh.
            Err(_) => serve_shell(config, request)?,
        },
        Reply::AppShell => serve_shell(config, request)?,
    }
    Ok(())
}

fn serve_shell(config: &Config, request: tiny_http::Request) -> Result<()> {
    let shell = config.ui_dir.as_ref().map(|d| d.join("index.html"));
    match shell.and_then(|p| std::fs::read(p).ok()) {
        Some(data) => {
            let header = "Content-Type: text/html; charset=utf-8"
                .parse::<tiny_http::Header>()
                .map_err(|_| anyhow::anyhow!("bad header"))?;
            request.respond(tiny_http::Response::from_data(data).with_header(header))?;
        }
        None => {
            request
                .respond(tiny_http::Response::from_string("Not found.").with_status_code(404))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Access, Token};

    fn get(url: &str) -> Request {
        Request {
            method: "GET".into(),
            url: url.into(),
            token: None,
            body: None,
        }
    }

    fn post(url: &str, body: Value) -> Request {
        Request {
            method: "POST".into(),
            url: url.into(),
            token: None,
            body: Some(body),
        }
    }

    fn loopback_with_ui() -> Config {
        Config::loopback(0).with_ui_dir("/app/dist")
    }

    fn status_of(reply: &Reply) -> u16 {
        match reply {
            Reply::Json { status, .. } => *status,
            _ => 0,
        }
    }

    // --- auth -----------------------------------------------------------

    #[test]
    fn loopback_needs_no_token() {
        let reply = handle(&loopback_with_ui(), &get("/"));
        assert!(matches!(reply, Reply::File { .. }));
    }

    #[test]
    fn lan_without_a_token_is_refused() {
        let mut config = Config::lan(0).with_ui_dir("/app/dist");
        config.token = Some(Token::from_string("thecorrecttoken"));
        assert_eq!(status_of(&handle(&config, &get("/"))), 401);
    }

    #[test]
    fn lan_with_the_right_token_is_allowed() {
        let mut config = Config::lan(0).with_ui_dir("/app/dist");
        config.token = Some(Token::from_string("thecorrecttoken"));
        let mut request = get("/");
        request.token = Some("thecorrecttoken".into());
        assert!(matches!(handle(&config, &request), Reply::File { .. }));
    }

    #[test]
    fn lan_with_a_wrong_token_is_refused() {
        let mut config = Config::lan(0).with_ui_dir("/app/dist");
        config.token = Some(Token::from_string("thecorrecttoken"));
        let mut request = get("/");
        request.token = Some("nottherighttoken".into());
        assert_eq!(status_of(&handle(&config, &request)), 401);
    }

    /// The UI shell itself must be behind auth. Serving it tells a stranger the
    /// machine is here and what it runs.
    #[test]
    fn an_unauthenticated_lan_request_is_told_nothing_about_the_machine() {
        let mut config = Config::lan(0).with_ui_dir("/app/dist");
        config.token = Some(Token::from_string("t"));
        for path in ["/", "/index.html", "/assets/index.js", "/api/list_tasks"] {
            let reply = handle(&config, &get(path));
            assert_eq!(status_of(&reply), 401, "{path} leaked past auth");
        }
    }

    /// A misconfiguration — LAN access with no token — must refuse everything
    /// rather than allow everything.
    #[test]
    fn lan_access_without_a_configured_token_fails_closed() {
        let config = Config {
            access: Access::Lan,
            port: 0,
            ui_dir: Some("/app/dist".into()),
            token: None,
        };
        assert_eq!(status_of(&handle(&config, &get("/"))), 401);
    }

    #[test]
    fn a_token_can_arrive_in_the_query_string() {
        assert_eq!(
            token_from_url("/index.html?token=abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            token_from_url("/?foo=1&token=xyz&bar=2"),
            Some("xyz".to_string())
        );
        assert_eq!(token_from_url("/index.html"), None);
        assert_eq!(token_from_url("/?nottoken=abc"), None);
    }

    // --- api routing ----------------------------------------------------

    #[test]
    fn a_known_command_is_marked_for_execution() {
        let reply = handle(&loopback_with_ui(), &post("/api/list_tasks", json!({})));
        assert!(is_command(&reply), "list_tasks should have been dispatched");
    }

    #[test]
    fn command_arguments_are_passed_through() {
        let reply = handle(
            &loopback_with_ui(),
            &post("/api/create_task", json!({ "name": "Accounting 302" })),
        );
        assert!(is_command(&reply));
        match reply {
            Reply::Json { body, .. } => assert_eq!(body["name"], "Accounting 302"),
            _ => panic!("expected the args to be carried"),
        }
    }

    #[test]
    fn an_unknown_command_is_a_404_not_a_dispatch() {
        let reply = handle(&loopback_with_ui(), &post("/api/rm_rf_slash", json!({})));
        assert!(!is_command(&reply));
        assert_eq!(status_of(&reply), 404);
    }

    /// A command must not be triggerable by a link. `GET /api/stop_capture`
    /// reachable from an `<img src>` would let any page the user visits stop
    /// their recording.
    #[test]
    fn commands_cannot_be_invoked_with_get() {
        for command in ["list_tasks", "stop_capture", "download_models"] {
            let reply = handle(&loopback_with_ui(), &get(&format!("/api/{command}")));
            assert_eq!(
                status_of(&reply),
                405,
                "{command} was reachable by GET, so a link could trigger it"
            );
            assert!(!is_command(&reply));
        }
    }

    #[test]
    fn a_command_with_no_body_still_dispatches_with_empty_args() {
        let request = Request {
            method: "POST".into(),
            url: "/api/list_tasks".into(),
            token: None,
            body: None,
        };
        let reply = handle(&loopback_with_ui(), &request);
        assert!(is_command(&reply));
    }

    #[test]
    fn a_query_string_does_not_break_command_routing() {
        let reply = handle(
            &loopback_with_ui(),
            &post("/api/list_tasks?token=x", json!({})),
        );
        assert!(is_command(&reply));
    }

    // --- static routing -------------------------------------------------

    #[test]
    fn the_root_serves_the_shell_file() {
        match handle(&loopback_with_ui(), &get("/")) {
            Reply::File { path } => assert!(path.ends_with("index.html")),
            other => panic!("expected a file, got {other:?}"),
        }
    }

    #[test]
    fn an_asset_path_resolves_to_a_file() {
        match handle(&loopback_with_ui(), &get("/assets/index-abc.js")) {
            Reply::File { path } => {
                assert!(path.ends_with("assets/index-abc.js"));
                assert!(path.starts_with("/app/dist"));
            }
            other => panic!("expected a file, got {other:?}"),
        }
    }

    /// Traversal must not reach `Reply::File` at all — the refusal happens in
    /// routing, before any I/O.
    #[test]
    fn a_traversal_attempt_never_becomes_a_file_reply() {
        for attack in ["/../../etc/passwd", "/..%2f..%2fetc%2fpasswd", r"/..\..\x"] {
            let reply = handle(&loopback_with_ui(), &get(attack));
            assert!(
                !matches!(reply, Reply::File { .. }),
                "{attack} reached the filesystem"
            );
            assert_eq!(status_of(&reply), 404);
        }
    }

    #[test]
    fn an_api_only_server_says_so_instead_of_serving_files() {
        let config = Config::loopback(0); // no ui_dir
        assert_eq!(status_of(&handle(&config, &get("/"))), 404);
    }
}
