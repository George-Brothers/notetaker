//! Is any OTHER process using the default microphone right now?
//!
//! WASAPI keeps a session per process on each device, each with a live state.
//! An `Active` capture session whose pid is not ours means some other app is
//! actually pulling audio from the mic — a call, in practice. Unlike the
//! macOS side (a device-wide flag), this can and does exclude our own
//! sessions, so our capture and level meter never look like a meeting.

use anyhow::{Context, Result};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::core::Interface;

/// True while another process has an active capture session on the default
/// microphone. Errors mean "could not ask", and the watcher reads them as a
/// quiet mic.
pub fn mic_in_use() -> Result<bool> {
    // Idempotent per thread; RPC_E_CHANGED_MODE just means the thread already
    // has COM in a different mode, which is fine for what we do here.
    // SAFETY: standard COM initialization; no aliasing is involved.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let our_pid = std::process::id();

    // SAFETY: every call below follows the documented WASAPI COM sequence and
    // checks its HRESULT through the windows crate's Result wrappers.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .context("creating the device enumerator")?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eCapture, eConsole)
            .context("no default microphone")?;
        let manager: IAudioSessionManager2 = device
            .Activate(CLSCTX_ALL, None)
            .context("opening the session manager")?;
        let sessions = manager
            .GetSessionEnumerator()
            .context("listing audio sessions")?;
        let count = sessions.GetCount().context("counting audio sessions")?;
        for i in 0..count {
            let Ok(control) = sessions.GetSession(i) else {
                continue;
            };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            let pid = control2.GetProcessId().unwrap_or(0);
            if pid == our_pid {
                continue;
            }
            if control.GetState().map(|s| s == AudioSessionStateActive).unwrap_or(false) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
