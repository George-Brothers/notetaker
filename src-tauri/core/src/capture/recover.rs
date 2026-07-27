//! Recovering recordings whose capture died mid-write: repair the WAV header,
//! finalize, and put them back in the queue.
//!
//! Implemented in Plan B1 Task 2.
