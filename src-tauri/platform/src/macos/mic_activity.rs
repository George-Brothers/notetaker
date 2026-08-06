//! Is anything on this Mac using the microphone right now?
//!
//! The HAL answers this in one property: `DeviceIsRunningSomewhere` on the
//! default input device is true while *any* process — this one included —
//! holds the device open. That "this one included" matters: the caller must
//! not read a hot mic as "a call started" while its own capture (or level
//! meter) is the thing holding it. The watcher suppresses the signal while a
//! recording runs; a meter's brief open is filtered by the watcher's
//! consecutive-poll debounce.
//!
//! No permission is involved: this reads a device property, not audio.

use anyhow::{anyhow, Result};
use objc2_core_audio::{
    kAudioDevicePropertyDeviceIsRunningSomewhere, kAudioHardwarePropertyDefaultInputDevice,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
    AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress,
};
use std::ffi::c_void;
use std::ptr::NonNull;

fn get_u32(object: AudioObjectID, selector: u32) -> Result<u32> {
    let address = AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the address points at a live u32 of the size we report, and the
    // HAL writes at most that many bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(&mut value as *mut u32 as *mut c_void).expect("stack address"),
        )
    };
    if status != 0 {
        return Err(anyhow!("CoreAudio refused property {selector}: OSStatus {status}"));
    }
    Ok(value)
}

/// True while any process holds the default input device open.
///
/// Errors mean "could not ask", not "no": a machine with no input device at
/// all returns an error, and the watcher treats that as a quiet mic rather
/// than a meeting.
pub fn mic_in_use() -> Result<bool> {
    let device = get_u32(
        kAudioObjectSystemObject as AudioObjectID,
        kAudioHardwarePropertyDefaultInputDevice,
    )?;
    if device == 0 {
        // kAudioObjectUnknown: no input device exists.
        return Ok(false);
    }
    Ok(get_u32(device, kAudioDevicePropertyDeviceIsRunningSomewhere)? != 0)
}
