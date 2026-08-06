//! Records from the microphone for a few seconds and reports what arrived.
//!
//! The companion to `system-audio`, and it exists for the same reason: no unit
//! test can open a real device, and CI runners have no microphone. Between them
//! these two examples are the only things that can answer the question this
//! project carried unanswered through three plans — *does a device actually
//! produce a sample through this code?*
//!
//! ```text
//! cargo run -p notetaker-platform --example microphone
//! ```
//!
//! Say something while it runs. A peak near zero means the device opened and
//! delivered silence, which is a different bug from failing to open.

fn main() {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        eprintln!("cpal is not built on this platform, so there is no microphone here.");
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        use notetaker_platform::MicSource;
        use std::time::{Duration, Instant};

        println!("Opening the microphone...");
        let mut source = match MicSource::start() {
            Ok(s) => s,
            Err(e) => {
                // On macOS the first run raises the microphone permission
                // dialog. A refusal lands here, and this is the message the
                // user would see.
                println!("\nCould not open it. This is what the user would be told:\n");
                println!("  {e:#}\n");
                std::process::exit(1);
            }
        };

        println!("Open: {}", source.label());
        println!("Listening for 5 seconds — say something now.\n");

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut total = 0usize;
        let mut peak = 0.0f32;
        let mut batch = Vec::new();

        while Instant::now() < deadline {
            batch.clear();
            if let Err(e) = source.read(&mut batch) {
                println!("read failed: {e:#}");
                break;
            }
            total += batch.len();
            for s in &batch {
                let a = s.abs();
                if a > peak {
                    peak = a;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let _ = source.stop();

        println!(
            "samples:  {total} (about {:.1}s at 16 kHz)",
            total as f32 / 16_000.0
        );
        println!("peak:     {peak:.4}");

        if total == 0 {
            println!("\nFAILED: the microphone opened but delivered nothing.");
            std::process::exit(2);
        }
        if peak < 0.001 {
            println!("\nOpened and delivered samples, but they are all but silent.");
            println!("If you spoke, check which input device the system is using.");
            std::process::exit(3);
        }
        println!("\nOK: the microphone produced real audio.");
    }
}
