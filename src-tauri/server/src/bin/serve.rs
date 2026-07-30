//! Runs Notetaker as a server: capture and processing on this machine, UI in a
//! browser.
//!
//! Useful in its own right, and useful because it needs no Tauri — so on a PC
//! this is a working Notetaker before the desktop shell is built, and on the
//! Linux development machine it is the only way to exercise the real runtime end
//! to end.
//!
//! ```text
//! notetaker-serve                    # http://127.0.0.1:4321, this computer only
//! notetaker-serve --lan              # reachable from your network, prints a code
//! notetaker-serve --port 8080
//! notetaker-serve --ui-dir ./dist
//! ```

use std::sync::Arc;

use anyhow::{Context, Result};
use notetaker_core::capture::platform::PlatformSources;
use notetaker_core::paths;
use notetaker_core::power::probe::default_probe;
use notetaker_core::runtime::Runtime;
use notetaker_server::{serve, Config};

const DEFAULT_PORT: u16 = 4321;

fn main() -> Result<()> {
    // `info` by default so the startup line, which carries the URL and the LAN
    // access code, is actually visible.
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    env_logger_init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    let lan = args.iter().any(|a| a == "--lan");
    let port = flag_value(&args, "--port")
        .map(|v| v.parse::<u16>().context("--port must be a number"))
        .transpose()?
        .unwrap_or(DEFAULT_PORT);

    let mut config = if lan {
        Config::lan(port)
    } else {
        Config::loopback(port)
    };

    // Default to a `dist/` next to the working directory, which is where a
    // `pnpm build` puts it.
    let ui_dir = flag_value(&args, "--ui-dir")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let d = std::path::PathBuf::from("dist");
            d.join("index.html").exists().then_some(d)
        });
    if let Some(dir) = ui_dir {
        log::info!("serving the interface from {}", dir.display());
        config = config.with_ui_dir(dir);
    } else {
        log::warn!(
            "no built interface found (looked for dist/index.html) — serving the API only. \
             Run `pnpm build` first, or pass --ui-dir."
        );
    }

    let storage_root = paths::default_storage_root()?;
    let app_dir = paths::default_app_dir()?;
    log::info!("library: {}", storage_root.display());

    let runtime = Runtime::open(
        &app_dir,
        &storage_root,
        Box::new(PlatformSources::new()),
        default_probe(),
    )
    .context("opening the Notetaker runtime")?;

    serve(config, Arc::new(runtime))
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).cloned()
}

fn print_help() {
    println!(
        "notetaker-serve — run Notetaker with its interface in a browser

    --lan             also reachable from other devices on your network.
                      Prints an access code that every request must carry.
    --port <number>   default {DEFAULT_PORT}
    --ui-dir <path>   where the built interface lives (default: ./dist)
    -h, --help        this message

Without --lan the server is reachable only from this computer."
    );
}

/// Minimal `env_logger` stand-in so this binary adds no dependency for the sake
/// of three log lines.
///
/// A `static` logger rather than a boxed one: `log::set_boxed_logger` needs the
/// crate's `std` feature, which core does not enable.
struct Simple;

impl log::Log for Simple {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}

static LOGGER: Simple = Simple;

fn env_logger_init() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}
