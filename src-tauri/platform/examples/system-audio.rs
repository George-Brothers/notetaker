//! Runs the macOS system-audio capture for a few seconds and reports what it
//! got. The one thing a unit test cannot do.
//!
//! `speaker.rs` is Objective-C glue: a class defined at runtime, a protocol
//! conformance, and selectors matched by string. All of that compiles cleanly
//! whether or not it is correct, and the failure modes are "crashes on the
//! first callback" or, worse, "receives nothing at all and records silence".
//! Neither is visible to `cargo test`, and neither is visible to CI — a GitHub
//! runner has no audio device and no Screen Recording grant.
//!
//! So this exists to be run by a person on a real Mac:
//!
//! ```text
//! cargo run -p notetaker-platform --example system-audio
//! ```
//!
//! Play something audible first. Expect either real sample counts, or the
//! plain-English permission message — both are correct outcomes, and both
//! prove the class registered and the selectors resolved. Silence with no
//! error is the failure this is looking for.

fn main() {
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("This example only does anything on macOS.");
    }

    #[cfg(target_os = "macos")]
    {
        use notetaker_platform::macos::speaker::SystemAudioSource;
        use std::time::{Duration, Instant};

        println!("Asking macOS to share this computer's sound...");
        let mut source = match SystemAudioSource::start() {
            Ok(s) => s,
            Err(e) => {
                // Not a crash: this is the designed path when Screen Recording
                // has not been granted, and the message is the whole user
                // experience of that case. Worth reading rather than just
                // failing.
                println!("\nCould not start. This is what the user would be told:\n");
                println!("  {e:#}\n");
                std::process::exit(1);
            }
        };

        println!("Started: {}", source.label());
        println!("Listening for 5 seconds — play something audible now.\n");

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

        // 5 seconds at 16 kHz is 80,000 samples. Far fewer means the stream
        // started and then starved, which is a different bug from never
        // starting at all.
        println!("samples:  {total} (about {:.1}s at 16 kHz)", total as f32 / 16_000.0);
        println!("peak:     {peak:.4}");
        println!("dropped:  {}", source.dropped_samples());

        if total == 0 {
            println!("\nFAILED: the stream started but delivered nothing.");
            std::process::exit(2);
        }
        if peak == 0.0 {
            println!("\nGot samples, but all of them are silence.");
            println!("If something was actually playing, the buffer layout is wrong.");
            std::process::exit(3);
        }
        println!("\nOK: real audio arrived from this computer's speakers.");
    }
}
