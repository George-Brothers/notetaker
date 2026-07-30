//! The bridge between a push-based OS audio callback and a pull-based `read`.
//!
//! Every real audio API on both platforms is push-based: it hands you a
//! callback on a high-priority thread and expects you to return immediately.
//! `notetaker_core::capture::source::AudioSource` is deliberately pull-based,
//! so that the session loop and every test of it stay synchronous and
//! deterministic. Its own doc comment names the fix — "each adapts by writing
//! into a ring buffer that `read` drains" — and this is that ring buffer.
//!
//! **The producer never blocks and never allocates.** Blocking inside an audio
//! callback does not cause a slow recording, it causes glitches across the
//! whole machine, and on some drivers it drops the stream entirely. So when the
//! ring is full this discards samples and counts them, and the count is
//! surfaced so the recording can carry a plain-English note about it — the
//! project's rule is that a problem with the *audio* outlives every processing
//! attempt, and silently short audio is exactly that.
//!
//! Portions adapted from anarlog (MIT, Copyright (c) 2023-present Fastrepl,
//! Inc.) — `crates/audio-actual/src/rt_ring.rs`. See the NOTICE file. Reworked
//! from their async waker-based reader to a plain synchronous drain, since
//! nothing above this is async.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

/// Ring capacity, in samples, used by the capture sources.
///
/// At the 48 kHz a device typically runs at, this is about ten seconds. Sized
/// for the gap between `read` calls, not for the callback: the session loop
/// polls far more often than this, so reaching the end of it means the loop has
/// stalled badly rather than that the buffer was a little small.
pub const DEFAULT_CAPACITY: usize = 480_000;

/// Creates a producer/consumer pair over a shared ring.
pub fn channel(capacity: usize) -> (RingWriter, RingReader) {
    let (prod, cons) = HeapRb::<f32>::new(capacity.max(1)).split();
    let dropped = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    (
        RingWriter {
            prod,
            dropped: dropped.clone(),
            finished: finished.clone(),
        },
        RingReader {
            cons,
            dropped,
            finished,
        },
    )
}

/// The callback-side half. Lives on the OS audio thread.
pub struct RingWriter {
    prod: HeapProd<f32>,
    dropped: Arc<AtomicUsize>,
    finished: Arc<AtomicBool>,
}

impl RingWriter {
    /// Writes what fits and counts what does not. Never blocks.
    ///
    /// Returns the number of samples written.
    pub fn write(&mut self, samples: &[f32]) -> usize {
        let pushed = self.prod.push_slice(samples);
        if pushed < samples.len() {
            self.dropped
                .fetch_add(samples.len() - pushed, Ordering::Relaxed);
        }
        pushed
    }

    /// Marks the stream permanently over — the device went away, the user
    /// stopped screen sharing, the capture thread hit an error it cannot
    /// recover from. The reader reports finished once it has drained what is
    /// left, so the last fragment of audio is never thrown away.
    pub fn finish(&self) {
        self.finished.store(true, Ordering::Release);
    }

    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// The `read`-side half. Lives on the session loop's thread.
pub struct RingReader {
    cons: HeapCons<f32>,
    dropped: Arc<AtomicUsize>,
    finished: Arc<AtomicBool>,
}

impl RingReader {
    /// Moves every available sample into `out`, appending. Returns how many.
    pub fn drain(&mut self, out: &mut Vec<f32>) -> usize {
        let available = self.cons.occupied_len();
        if available == 0 {
            return 0;
        }
        let start = out.len();
        out.resize(start + available, 0.0);
        let got = self.cons.pop_slice(&mut out[start..]);
        out.truncate(start + got);
        got
    }

    /// True once the writer has finished *and* the ring is empty.
    ///
    /// The ordering matters: reporting finished while samples remain would
    /// have the session stop and finalize a recording with its last fraction
    /// of a second still sitting in the buffer.
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire) && self.cons.occupied_len() == 0
    }

    /// Samples the producer had to discard because the ring was full.
    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Samples waiting to be read.
    pub fn available(&self) -> usize {
        self.cons.occupied_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_come_out_in_the_order_they_went_in() {
        let (mut w, mut r) = channel(64);
        w.write(&[1.0, 2.0, 3.0]);
        let mut out = Vec::new();
        assert_eq!(r.drain(&mut out), 3);
        assert_eq!(out, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn drain_appends_rather_than_replacing() {
        let (mut w, mut r) = channel(64);
        w.write(&[7.0]);
        let mut out = vec![1.0, 2.0];
        r.drain(&mut out);
        assert_eq!(out, vec![1.0, 2.0, 7.0]);
    }

    #[test]
    fn draining_an_empty_ring_yields_nothing_and_leaves_out_untouched() {
        let (_w, mut r) = channel(64);
        let mut out = vec![5.0];
        assert_eq!(r.drain(&mut out), 0);
        assert_eq!(out, vec![5.0], "must not resize on an empty drain");
    }

    #[test]
    fn writes_across_several_calls_read_back_as_one_stream() {
        let (mut w, mut r) = channel(64);
        w.write(&[1.0, 2.0]);
        w.write(&[3.0]);
        w.write(&[4.0, 5.0]);
        let mut out = Vec::new();
        r.drain(&mut out);
        assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn interleaved_writes_and_drains_lose_nothing() {
        let (mut w, mut r) = channel(8);
        let mut out = Vec::new();
        for i in 0..100 {
            w.write(&[i as f32]);
            r.drain(&mut out);
        }
        assert_eq!(out.len(), 100);
        assert_eq!(r.dropped(), 0);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[99], 99.0);
    }

    // --- overflow -------------------------------------------------------

    /// The producer must never block, so a full ring means discarded samples.
    /// The count is the whole point: it is what lets the recording tell the
    /// user its audio is short instead of silently being wrong.
    #[test]
    fn a_full_ring_drops_samples_and_counts_them_instead_of_blocking() {
        let (mut w, mut r) = channel(4);
        let written = w.write(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(written, 4, "should write exactly what fits");
        assert_eq!(w.dropped(), 2);
        assert_eq!(r.dropped(), 2, "the count must be visible to the reader");

        let mut out = Vec::new();
        r.drain(&mut out);
        assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn dropped_count_accumulates_across_writes() {
        let (mut w, r) = channel(2);
        w.write(&[1.0, 2.0, 3.0]); // drops 1
        w.write(&[4.0, 5.0]); // ring already full, drops 2
        assert_eq!(r.dropped(), 3);
    }

    #[test]
    fn no_drops_reported_when_everything_fits() {
        let (mut w, r) = channel(1024);
        w.write(&vec![0.5; 1000]);
        assert_eq!(r.dropped(), 0);
    }

    #[test]
    fn a_zero_capacity_request_still_yields_a_usable_ring() {
        let (mut w, mut r) = channel(0);
        // Must not panic. Capacity is clamped to at least 1.
        w.write(&[1.0]);
        let mut out = Vec::new();
        r.drain(&mut out);
        assert_eq!(out, vec![1.0]);
    }

    // --- finishing ------------------------------------------------------

    #[test]
    fn a_fresh_ring_is_not_finished() {
        let (_w, r) = channel(8);
        assert!(!r.is_finished());
    }

    /// The ordering rule: buffered audio must be readable *before* the reader
    /// admits to being finished, or the session finalizes a recording that is
    /// missing its tail.
    #[test]
    fn not_finished_until_buffered_audio_has_been_drained() {
        let (mut w, mut r) = channel(8);
        w.write(&[1.0, 2.0]);
        w.finish();
        assert!(
            !r.is_finished(),
            "reported finished while audio was still buffered"
        );

        let mut out = Vec::new();
        r.drain(&mut out);
        assert_eq!(out, vec![1.0, 2.0]);
        assert!(r.is_finished(), "should be finished once drained");
    }

    #[test]
    fn finishing_an_empty_ring_is_immediately_finished() {
        let (w, r) = channel(8);
        w.finish();
        assert!(r.is_finished());
    }

    #[test]
    fn available_reports_what_is_waiting() {
        let (mut w, mut r) = channel(16);
        assert_eq!(r.available(), 0);
        w.write(&[1.0, 2.0, 3.0]);
        assert_eq!(r.available(), 3);
        let mut out = Vec::new();
        r.drain(&mut out);
        assert_eq!(r.available(), 0);
    }

    // --- threading ------------------------------------------------------

    /// The real shape of the thing: a producer thread writing while the
    /// consumer drains, as the OS callback and the session loop actually do.
    #[test]
    fn survives_a_real_producer_thread() {
        let (mut w, mut r) = channel(DEFAULT_CAPACITY);
        let total = 100_000;
        let writer = std::thread::spawn(move || {
            for i in 0..total {
                w.write(&[i as f32]);
            }
            w.finish();
        });

        let mut out = Vec::new();
        while !r.is_finished() {
            r.drain(&mut out);
            std::thread::yield_now();
        }
        writer.join().unwrap();
        r.drain(&mut out);

        assert_eq!(r.dropped(), 0, "ring was too small for a steady producer");
        assert_eq!(out.len(), total);
        // Order must be preserved exactly, with nothing duplicated or skipped.
        for (i, s) in out.iter().enumerate() {
            assert_eq!(*s, i as f32, "sample {i} out of order");
        }
    }
}
