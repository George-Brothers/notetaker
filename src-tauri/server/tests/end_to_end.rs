//! End-to-end: a real socket, a real `Runtime`, real HTTP.
//!
//! The unit tests in `router` and `statics` cover the decisions; this covers the
//! wiring between them, which is the part that a pure test cannot reach. It
//! binds an actual port, starts an actual server thread, and speaks HTTP over
//! TCP by hand — no HTTP client dependency, so there is nothing between the
//! assertion and the bytes on the wire.
//!
//! This is also the piece of the cross-platform work that is *fully* verified on
//! the development machine rather than compile-checked and left for CI: the
//! served UI needs no microphone, no models and no OS-specific API, so it can be
//! genuinely exercised here.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notetaker_core::power::probe::FakeProbe;
use notetaker_core::power::PowerState;
use notetaker_core::runtime::{FakeSources, Runtime};
use notetaker_server::{serve, Access, Config, Token};

/// Asks the OS for a free port, then releases it. A fixed port would make the
/// suite fail when anything else on the machine happens to hold it.
fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("no free port");
    l.local_addr().unwrap().port()
}

fn test_runtime(dir: &std::path::Path) -> Arc<Runtime> {
    Arc::new(
        Runtime::open(
            &dir.join("app"),
            &dir.join("Notetaker"),
            Box::new(FakeSources { secs: 0.2 }),
            Arc::new(FakeProbe {
                state: Some(PowerState {
                    idle_secs: 9_000,
                    on_ac: true,
                    battery_pct: Some(90),
                }),
            }),
        )
        .expect("runtime must open on a fresh directory"),
    )
}

/// Starts a server on a background thread and waits until it accepts.
fn start(config: Config, runtime: Arc<Runtime>) -> u16 {
    let port = config.port;
    std::thread::spawn(move || {
        let _ = serve(config, runtime);
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return port;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("server never came up on port {port}");
}

/// Sends a raw request and returns `(status, body)`.
fn request(port: u16, raw: &str) -> (u16, String) {
    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).expect("could not connect to the test server");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    stream.flush().unwrap();

    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);

    let status = response
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("could not parse a status line out of: {response:?}"));
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

fn post(port: u16, path: &str, json: &str, token: Option<&str>) -> (u16, String) {
    let token_header = token
        .map(|t| format!("X-Notetaker-Token: {t}\r\n"))
        .unwrap_or_default();
    request(
        port,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n{token_header}Connection: close\r\n\r\n{json}",
            json.len()
        ),
    )
}

fn get(port: u16, path: &str) -> (u16, String) {
    request(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"),
    )
}

// -------------------------------------------------------------------------
// The API, over a real socket
// -------------------------------------------------------------------------

#[test]
fn a_command_round_trips_over_http() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(Config::loopback(free_port()), test_runtime(dir.path()));

    let (status, body) = post(port, "/api/list_tasks", "{}", None);
    assert_eq!(status, 200, "body was {body}");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("not JSON");
    assert!(
        parsed.is_array(),
        "list_tasks should return an array: {body}"
    );
}

/// The real test of the whole idea: a command that changes state, then reading
/// that state back through a second request. If the served UI can do this it can
/// run the library.
#[test]
fn a_task_created_over_http_is_visible_to_the_next_request() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(Config::loopback(free_port()), test_runtime(dir.path()));

    let (status, body) = post(
        port,
        "/api/create_task",
        r#"{"name":"Accounting 302"}"#,
        None,
    );
    assert_eq!(status, 200, "create_task failed: {body}");

    let (_, body) = post(port, "/api/list_tasks", "{}", None);
    assert!(
        body.contains("Accounting 302"),
        "the task did not persist: {body}"
    );
}

#[test]
fn settings_round_trip_over_http() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(Config::loopback(free_port()), test_runtime(dir.path()));

    let (status, body) = post(port, "/api/get_settings", "{}", None);
    assert_eq!(status, 200, "{body}");
    let mut settings: serde_json::Value = serde_json::from_str(&body).unwrap();
    settings["llmModel"] = serde_json::json!("qwen3:14b");

    let (status, body) = post(
        port,
        "/api/set_settings",
        &serde_json::json!({ "settings": settings }).to_string(),
        None,
    );
    assert_eq!(status, 200, "set_settings failed: {body}");

    let (_, body) = post(port, "/api/get_settings", "{}", None);
    assert!(
        body.contains("qwen3:14b"),
        "the setting did not persist: {body}"
    );
}

/// An error must arrive as the runtime's own plain-English message, not a
/// stack trace or a bare 500.
#[test]
fn a_failing_command_returns_a_readable_error() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(Config::loopback(free_port()), test_runtime(dir.path()));

    let (status, body) = post(port, "/api/get_recording", r#"{"id":"nope"}"#, None);
    assert_eq!(status, 400, "{body}");
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        parsed.get("error").and_then(|e| e.as_str()).is_some(),
        "no error message in {body}"
    );
}

#[test]
fn a_missing_argument_is_reported_rather_than_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(Config::loopback(free_port()), test_runtime(dir.path()));

    let (status, body) = post(port, "/api/create_task", "{}", None);
    assert_eq!(status, 400, "a missing argument should not succeed: {body}");
    assert!(
        body.contains("name"),
        "the error should name the argument: {body}"
    );
}

#[test]
fn an_unknown_command_is_a_404() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(Config::loopback(free_port()), test_runtime(dir.path()));
    let (status, _) = post(port, "/api/definitely_not_real", "{}", None);
    assert_eq!(status, 404);
}

/// A command must not be reachable by a link — otherwise any page the user
/// visits could stop their recording with an `<img src>`.
#[test]
fn a_command_cannot_be_triggered_by_a_get() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(Config::loopback(free_port()), test_runtime(dir.path()));
    let (status, _) = get(port, "/api/stop_capture");
    assert_eq!(status, 405);
}

// -------------------------------------------------------------------------
// Static assets
// -------------------------------------------------------------------------

#[test]
fn the_ui_is_served_and_traversal_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let ui = dir.path().join("dist");
    std::fs::create_dir_all(ui.join("assets")).unwrap();
    std::fs::write(
        ui.join("index.html"),
        "<!doctype html><title>Notetaker</title>",
    )
    .unwrap();
    std::fs::write(ui.join("assets/app.js"), "console.log(1)").unwrap();
    // The file an attacker would want, one level above the served root.
    std::fs::write(dir.path().join("secrets.txt"), "SECRET").unwrap();

    let port = start(
        Config::loopback(free_port()).with_ui_dir(&ui),
        test_runtime(dir.path()),
    );

    let (status, body) = get(port, "/");
    assert_eq!(status, 200);
    assert!(body.contains("Notetaker"), "shell not served: {body}");

    let (status, body) = get(port, "/assets/app.js");
    assert_eq!(status, 200);
    assert!(body.contains("console.log"), "asset not served: {body}");

    // The whole reason `statics::resolve` is strict.
    for attack in [
        "/../secrets.txt",
        "/assets/../../secrets.txt",
        "/..%2fsecrets.txt",
    ] {
        let (_, body) = get(port, attack);
        assert!(
            !body.contains("SECRET"),
            "{attack} leaked a file outside the served directory"
        );
    }
}

/// A client-side route must survive a hard refresh: `/settings` is not a file,
/// and must come back as the app shell rather than a 404.
#[test]
fn a_client_side_route_falls_back_to_the_app_shell() {
    let dir = tempfile::tempdir().unwrap();
    let ui = dir.path().join("dist");
    std::fs::create_dir_all(&ui).unwrap();
    std::fs::write(
        ui.join("index.html"),
        "<!doctype html><title>Notetaker</title>",
    )
    .unwrap();

    let port = start(
        Config::loopback(free_port()).with_ui_dir(&ui),
        test_runtime(dir.path()),
    );

    let (status, body) = get(port, "/settings");
    assert_eq!(status, 200, "a client-side route 404'd: {body}");
    assert!(body.contains("Notetaker"), "{body}");
}

// -------------------------------------------------------------------------
// LAN access
// -------------------------------------------------------------------------

/// The security property, over a real socket: with LAN access on, nothing is
/// served without the token — not the API, and not the UI shell either.
#[test]
fn lan_access_refuses_everything_without_the_token() {
    let dir = tempfile::tempdir().unwrap();
    let ui = dir.path().join("dist");
    std::fs::create_dir_all(&ui).unwrap();
    std::fs::write(
        ui.join("index.html"),
        "<!doctype html><title>Notetaker</title>",
    )
    .unwrap();

    let token = Token::from_string("the-correct-access-code");
    let mut config = Config::lan(free_port()).with_ui_dir(&ui);
    config.token = Some(token.clone());
    assert_eq!(config.access, Access::Lan);
    let port = start(config, test_runtime(dir.path()));

    // No token.
    let (status, _) = get(port, "/");
    assert_eq!(status, 401, "the UI shell was served without a token");
    let (status, _) = post(port, "/api/list_tasks", "{}", None);
    assert_eq!(status, 401, "the API answered without a token");

    // Wrong token.
    let (status, _) = post(port, "/api/list_tasks", "{}", Some("wrong-code"));
    assert_eq!(status, 401);

    // Right token, in the header.
    let (status, body) = post(
        port,
        "/api/list_tasks",
        "{}",
        Some("the-correct-access-code"),
    );
    assert_eq!(status, 200, "the correct token was refused: {body}");

    // Right token, in the query string — how it arrives from a phone.
    let (status, body) = get(port, "/?token=the-correct-access-code");
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("Notetaker"));
}

/// Loopback is the default and needs no token. Pinned so the convenience of
/// local use cannot be quietly turned into the default for network use.
#[test]
fn loopback_serves_without_a_token() {
    let dir = tempfile::tempdir().unwrap();
    let port = start(Config::loopback(free_port()), test_runtime(dir.path()));
    let (status, _) = post(port, "/api/list_tasks", "{}", None);
    assert_eq!(status, 200);
}
