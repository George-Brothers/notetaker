//! Live check for the mic-in-use probe. Run it twice: once with everything
//! quiet (expect false), once during a call or voice memo (expect true).
//! No CI runner has audio hardware, so like the capture examples this is a
//! human-at-a-real-machine check.

fn main() {
    #[cfg(target_os = "macos")]
    println!(
        "mic in use: {:?}",
        notetaker_platform::macos::mic_activity::mic_in_use()
    );
    #[cfg(target_os = "windows")]
    println!(
        "mic in use: {:?}",
        notetaker_platform::windows::mic_activity::mic_in_use()
    );
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    println!("no probe on this OS");
}
