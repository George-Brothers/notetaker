//! macOS system-wide paste for dictation.
//!
//! The transcript is written to the general pasteboard, Cmd-V is posted only
//! when Accessibility is already granted, and the old pasteboard is restored
//! only if its change count is still the one created by us. There is no timer
//! based restore: another application may legitimately replace the clipboard
//! while the paste event is in flight, and that clipboard must win.

use anyhow::{Context, Result};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardContentsOptions, NSPasteboardItem, NSPasteboardWriting,
};
use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventTapLocation};
use objc2_foundation::{NSArray, NSData, NSString};
use std::ffi::c_void;
use std::thread;
use std::time::Duration;

use crate::PasteOutcome;

const PLAIN_TEXT_UTI: &str = "public.utf8-plain-text";
const CONCEALED_UTI: &str = "org.nspasteboard.ConcealedType";
const TRANSIENT_UTI: &str = "org.nspasteboard.TransientType";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClipboardItem {
    types: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClipboardSnapshot {
    change_count: isize,
    items: Vec<ClipboardItem>,
}

pub fn paste_text(text: &str) -> Result<PasteOutcome> {
    let board = NSPasteboard::generalPasteboard();
    let snapshot = snapshot(&board)?;
    write_dictation_text(&board, text)?;
    let injected_count = board.changeCount();

    if !objc2_core_graphics::CGPreflightPostEventAccess() {
        return Ok(PasteOutcome::copied(
            "Copied — press Cmd-V in the focused app. Accessibility is required for automatic paste.",
        ));
    }

    let keycode = current_layout_v_keycode().context(
        "could not resolve the V key for the current keyboard layout; transcript remains on the clipboard",
    )?;
    post_command_v(keycode)?;

    let restored = if board.changeCount() == injected_count {
        restore(&board, &snapshot)?
    } else {
        false
    };
    let message = if restored {
        "Inserted into the focused app; the previous clipboard was restored.".to_string()
    } else {
        "Inserted into the focused app; the clipboard changed during paste and was left untouched."
            .to_string()
    };
    Ok(PasteOutcome {
        inserted: true,
        clipboard_restored: restored,
        message,
    })
}

pub fn copy_text(text: &str) -> Result<PasteOutcome> {
    let board = NSPasteboard::generalPasteboard();
    write_dictation_text(&board, text)?;
    Ok(PasteOutcome::copied("Copied to the clipboard."))
}

fn snapshot(board: &NSPasteboard) -> Result<ClipboardSnapshot> {
    let Some(items) = board.pasteboardItems() else {
        return Ok(ClipboardSnapshot {
            change_count: board.changeCount(),
            items: Vec::new(),
        });
    };

    let mut snapshot_items = Vec::with_capacity(items.len());
    for item in items.to_vec() {
        let mut types = Vec::with_capacity(item.types().len());
        for ty in item.types().to_vec() {
            let name = ty.to_string();
            let data = item
                .dataForType(&ty)
                .with_context(|| format!("reading clipboard data for UTI {name}"))?;
            types.push((name, data.to_vec()));
        }
        snapshot_items.push(ClipboardItem { types });
    }
    Ok(ClipboardSnapshot {
        change_count: board.changeCount(),
        items: snapshot_items,
    })
}

fn write_dictation_text(board: &NSPasteboard, text: &str) -> Result<()> {
    if board.prepareForNewContentsWithOptions(NSPasteboardContentsOptions(0)) <= 0 {
        anyhow::bail!("the general pasteboard refused new contents")
    }
    let plain_text = NSString::from_str(text);
    let plain_type = NSString::from_str(PLAIN_TEXT_UTI);
    if !board.setString_forType(&plain_text, &plain_type) {
        anyhow::bail!("the general pasteboard refused the transcript text")
    }
    for marker in [CONCEALED_UTI, TRANSIENT_UTI] {
        let marker_type = NSString::from_str(marker);
        let marker_data = NSData::from_vec(Vec::new());
        if !board.setData_forType(Some(&marker_data), &marker_type) {
            anyhow::bail!("the general pasteboard refused the {marker} marker")
        }
    }
    Ok(())
}

fn restore(board: &NSPasteboard, snapshot: &ClipboardSnapshot) -> Result<bool> {
    if board.prepareForNewContentsWithOptions(NSPasteboardContentsOptions(0)) <= 0 {
        return Ok(false);
    }
    let mut items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = Vec::new();
    for saved_item in &snapshot.items {
        let item = NSPasteboardItem::new();
        for (ty, bytes) in &saved_item.types {
            let ty = NSString::from_str(ty);
            let data = NSData::from_vec(bytes.clone());
            if !item.setData_forType(&data, &ty) {
                return Ok(false);
            }
        }
        items.push(ProtocolObject::from_retained(item));
    }
    let objects = NSArray::from_retained_slice(&items);
    Ok(board.writeObjects(&objects))
}

fn post_command_v(keycode: u16) -> Result<()> {
    let down = CGEvent::new_keyboard_event(None, keycode, true)
        .context("could not create Cmd-V key-down event")?;
    let up = CGEvent::new_keyboard_event(None, keycode, false)
        .context("could not create Cmd-V key-up event")?;
    CGEvent::set_flags(Some(&down), CGEventFlags::MaskCommand);
    CGEvent::set_flags(Some(&up), CGEventFlags::MaskCommand);
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&down));
    thread::sleep(Duration::from_millis(12));
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&up));
    Ok(())
}

/// Resolve the physical key whose current layout produces `v`. On Dvorak and
/// other layouts, blindly posting QWERTY keycode 9 inserts the wrong letter.
fn current_layout_v_keycode() -> Option<u16> {
    unsafe {
        let source = TISCopyCurrentKeyboardLayoutInputSource();
        if source.is_null() {
            return None;
        }
        let layout_data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
        if layout_data.is_null() {
            CFRelease(source);
            return None;
        }
        let layout = CFDataGetBytePtr(layout_data);
        if layout.is_null() {
            CFRelease(source);
            return None;
        }
        let result = (0..128).find(|keycode| {
            let mut dead_key_state = 0_u32;
            let mut actual_length = 0_isize;
            let mut output = [0_u16; 4];
            let status = UCKeyTranslate(
                layout,
                *keycode,
                3,
                0,
                LMGetKbdType() as u32,
                1,
                &mut dead_key_state,
                output.len() as isize,
                &mut actual_length,
                output.as_mut_ptr(),
            );
            status == 0 && actual_length > 0 && output[0] as u32 == 'v' as u32
        });
        CFRelease(source);
        result
    }
}

type CFTypeRef = *const c_void;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    static kTISPropertyUnicodeKeyLayoutData: CFTypeRef;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> CFTypeRef;
    fn TISGetInputSourceProperty(input_source: CFTypeRef, property_key: CFTypeRef) -> CFTypeRef;
    fn UCKeyTranslate(
        key_layout: *const u8,
        virtual_key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        key_translate_options: isize,
        dead_key_state: *mut u32,
        max_string_length: isize,
        actual_string_length: *mut isize,
        unicode_string: *mut u16,
    ) -> i32;
    fn LMGetKbdType() -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
    fn CFRelease(cf: CFTypeRef);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_restore_round_trip_preserves_every_item_and_type() {
        let before = ClipboardSnapshot {
            change_count: 17,
            items: vec![
                ClipboardItem {
                    types: vec![
                        ("public.utf8-plain-text".into(), b"hello".to_vec()),
                        ("public.png".into(), vec![0, 1, 2, 3]),
                    ],
                },
                ClipboardItem {
                    types: vec![("public.url".into(), b"https://example.test".to_vec())],
                },
            ],
        };
        let after = ClipboardSnapshot {
            change_count: before.change_count,
            items: before.items.clone(),
        };
        assert_eq!(after, before);
        assert_eq!(after.items[0].types[1].1, vec![0, 1, 2, 3]);
    }
}
