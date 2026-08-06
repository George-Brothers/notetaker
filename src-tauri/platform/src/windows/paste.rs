//! Windows system-wide paste for dictation.
//!
//! The clipboard sequence number is the restore guard. If the focused
//! application or another clipboard owner changes the clipboard after we
//! inject the transcript, we leave that newer clipboard alone.

use anyhow::{Context, Result};
use enigo::{Direction, Enigo, InputResult, Key, Keyboard, Settings as EnigoSettings};
use std::mem::size_of;
use std::ptr::copy_nonoverlapping;

use crate::PasteOutcome;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};

const CF_UNICODETEXT: u32 = 13;

pub fn paste_text(text: &str) -> Result<PasteOutcome> {
    let previous = snapshot_text()?;
    write_text(text)?;
    let injected_sequence = unsafe { GetClipboardSequenceNumber() };

    let mut enigo_settings = EnigoSettings::default();
    enigo_settings.open_prompt_to_get_permissions = false;
    let mut enigo = Enigo::new(&enigo_settings).context("creating Windows input synthesizer")?;
    if let Err(error) = send_ctrl_v(&mut enigo) {
        return Ok(PasteOutcome::copied(format!(
            "Copied — press Ctrl-V in the focused app. Windows input injection failed: {error}"
        )));
    }

    let restored = if unsafe { GetClipboardSequenceNumber() } == injected_sequence {
        restore_text(previous.as_deref())?
    } else {
        false
    };
    Ok(PasteOutcome {
        inserted: true,
        clipboard_restored: restored,
        message: if restored {
            "Inserted into the focused app; the previous clipboard was restored.".into()
        } else {
            "Inserted into the focused app; the clipboard changed during paste and was left untouched.".into()
        },
    })
}

pub fn copy_text(text: &str) -> Result<PasteOutcome> {
    write_text(text)?;
    Ok(PasteOutcome::copied("Copied to the clipboard."))
}

fn send_ctrl_v(enigo: &mut Enigo) -> InputResult<()> {
    enigo.key(Key::Control, Direction::Press)?;
    let result = enigo.key(Key::V, Direction::Click);
    let release = enigo.key(Key::Control, Direction::Release);
    result.and(release)
}

fn snapshot_text() -> Result<Option<Vec<u16>>> {
    unsafe {
        OpenClipboard(None).context("opening the Windows clipboard")?;
        let result = (|| {
            let handle = match GetClipboardData(CF_UNICODETEXT) {
                Ok(handle) => handle,
                Err(_) => return Ok(None),
            };
            let memory = HGLOBAL(handle.0);
            let size = GlobalSize(memory);
            if size == 0 {
                return Ok(None);
            }
            let pointer = GlobalLock(memory).cast::<u16>();
            if pointer.is_null() {
                return Ok(None);
            }
            let count = size / size_of::<u16>();
            let data = std::slice::from_raw_parts(pointer, count)
                .iter()
                .copied()
                .take_while(|unit| *unit != 0)
                .chain(std::iter::once(0))
                .collect();
            let _ = GlobalUnlock(memory);
            Ok(Some(data))
        })();
        let _ = CloseClipboard();
        result
    }
}

fn write_text(text: &str) -> Result<()> {
    let mut utf16 = text.encode_utf16().collect::<Vec<_>>();
    utf16.push(0);
    unsafe {
        OpenClipboard(None).context("opening the Windows clipboard")?;
        let result = (|| {
            EmptyClipboard().context("clearing the Windows clipboard")?;
            let bytes = utf16.len() * size_of::<u16>();
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes)
                .context("allocating Windows clipboard memory")?;
            let pointer = GlobalLock(memory).cast::<u16>();
            if pointer.is_null() {
                anyhow::bail!("locking Windows clipboard memory")
            }
            copy_nonoverlapping(utf16.as_ptr(), pointer, utf16.len());
            let _ = GlobalUnlock(memory);
            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0)))
                .context("writing Unicode text to the Windows clipboard")?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

fn restore_text(previous: Option<&[u16]>) -> Result<bool> {
    unsafe {
        OpenClipboard(None).context("reopening the Windows clipboard")?;
        let result = (|| {
            EmptyClipboard().context("clearing the injected transcript")?;
            let Some(previous) = previous else {
                return Ok(true);
            };
            let bytes = previous.len() * size_of::<u16>();
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes)
                .context("allocating Windows clipboard restore memory")?;
            let pointer = GlobalLock(memory).cast::<u16>();
            if pointer.is_null() {
                anyhow::bail!("locking Windows clipboard restore memory")
            }
            copy_nonoverlapping(previous.as_ptr(), pointer, previous.len());
            let _ = GlobalUnlock(memory);
            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0)))
                .context("restoring Unicode text to the Windows clipboard")?;
            Ok(true)
        })();
        let _ = CloseClipboard();
        result
    }
}
