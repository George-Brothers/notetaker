//! The microphone, on both shipping platforms.
//!
//! `cpal` already abstracts WASAPI and CoreAudio, so unlike system audio this
//! needs no per-OS split — the same code covers Mac and PC. What it does need
//! is the same treatment as everything else here: the device picks the sample
//! rate and channel count, and both get normalized to mono at
//! [`TARGET_SAMPLE_RATE`] through tested code rather than inside the callback.
//!
//! Portions adapted from anarlog (MIT, Copyright (c) 2023-present Fastrepl,
//! Inc.) — `crates/audio-actual/src/mic.rs`. See the NOTICE file.
//!
//! Compile-verified for `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`
//! from Linux. `cpal` is not built on Linux at all (its `alsa-sys` needs
//! pkg-config we have no sudo for), so this module is absent there and the
//! capture logic above it is tested against core's `FakeSource` instead.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::convert::interleaved_f32_to_mono;
use crate::resample::Resampler;
use crate::ring::{self, RingReader};
use crate::TARGET_SAMPLE_RATE;

/// The microphone as a pull-based source.
pub struct MicSource {
    reader: RingReader,
    resampler: Resampler,
    /// Held to keep the stream alive — dropping it closes the device.
    stream: Option<cpal::Stream>,
    /// Set by the error callback when the OS reports the device is gone.
    device_lost: Arc<AtomicBool>,
    label: String,
}

impl MicSource {
    /// Opens the system default input device and starts capturing.
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No microphone is available on this computer.")?;
        let name = device
            .name()
            .unwrap_or_else(|_| "the microphone".to_string());
        let config = device
            .default_input_config()
            .with_context(|| format!("Notetaker could not read the settings for {name}."))?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let sample_format = config.sample_format();

        let (mut writer, reader) = ring::channel(ring::DEFAULT_CAPACITY);
        let device_lost = Arc::new(AtomicBool::new(false));

        let err_lost = device_lost.clone();
        let on_error = move |e: cpal::StreamError| {
            log::warn!("microphone stream error: {e}");
            err_lost.store(true, Ordering::Release);
        };

        // One closure per sample format cpal may hand us. Each does the same
        // two steps: widen to `f32`, then downmix to mono. The arithmetic lives
        // in `convert`, which is tested; nothing here touches samples except to
        // widen them. Buffers are declared per arm and reused across callbacks,
        // so a steady stream does not allocate on the audio thread.
        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let mut mono = Vec::new();
                device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        mono.clear();
                        interleaved_f32_to_mono(data, channels, &mut mono);
                        writer.write(&mono);
                    },
                    on_error,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let (mut wide, mut mono) = (Vec::new(), Vec::new());
                device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        wide.clear();
                        // 32768, not 32767, so i16::MIN maps to exactly -1.0.
                        wide.extend(data.iter().map(|s| *s as f32 / 32_768.0));
                        mono.clear();
                        interleaved_f32_to_mono(&wide, channels, &mut mono);
                        writer.write(&mono);
                    },
                    on_error,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let (mut wide, mut mono) = (Vec::new(), Vec::new());
                device.build_input_stream(
                    &config.into(),
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        wide.clear();
                        // u16 is offset binary: 32768 is silence, not zero.
                        // Getting this wrong yields a colossal DC offset, which
                        // sounds like silence but wrecks every level check
                        // downstream.
                        wide.extend(data.iter().map(|s| (*s as f32 - 32_768.0) / 32_768.0));
                        mono.clear();
                        interleaved_f32_to_mono(&wide, channels, &mut mono);
                        writer.write(&mono);
                    },
                    on_error,
                    None,
                )
            }
            other => {
                anyhow::bail!("{name} uses an audio format Notetaker cannot read yet ({other:?}).")
            }
        }
        .with_context(|| format!("Notetaker could not start recording from {name}."))?;

        stream
            .play()
            .with_context(|| format!("Notetaker could not start {name}."))?;

        log::info!(
            "microphone: {name}, {sample_rate} Hz, {channels} channel(s), {sample_format:?}"
        );

        Ok(Self {
            reader,
            resampler: Resampler::new(sample_rate, TARGET_SAMPLE_RATE)?,
            stream: Some(stream),
            device_lost,
            label: name,
        })
    }

    /// Appends whatever audio is ready, resampled to [`TARGET_SAMPLE_RATE`].
    pub fn read(&mut self, out: &mut Vec<f32>) -> Result<()> {
        let mut raw = Vec::new();
        self.reader.drain(&mut raw);
        self.resampler.push(&raw);
        self.resampler.drain(out)?;
        Ok(())
    }

    /// True once the device is gone and everything buffered has been read.
    ///
    /// Checks the buffer before reporting finished: a microphone unplugged
    /// mid-sentence should still contribute the words it did capture.
    pub fn is_finished(&self) -> bool {
        if self.device_lost.load(Ordering::Acquire) && self.reader.available() == 0 {
            return true;
        }
        self.reader.is_finished()
    }

    /// Closes the device.
    pub fn stop(&mut self) -> Result<()> {
        // Dropping the stream is what actually releases the microphone, and
        // pausing first makes sure no callback is mid-write when it goes.
        if let Some(stream) = self.stream.take() {
            let _ = stream.pause();
            drop(stream);
        }
        Ok(())
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Samples discarded because the session loop could not keep up.
    pub fn dropped_samples(&self) -> usize {
        self.reader.dropped()
    }
}

impl Drop for MicSource {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
