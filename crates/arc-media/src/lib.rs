//! Arcline media pipeline (docs/04).
//!
//! - [`libav`]: in-process, frame-accurate decode through a narrow C shim (preview and export).
//! - [`probe`]: `ffprobe`/`ffmpeg` child processes for probing and the import check.
//! - [`encode`]: export encoding through an `ffmpeg` child, with capability-probed encoders.
//! - [`fixtures`]: generated test media.

pub mod encode;
pub mod fixtures;
pub mod libav;
pub mod paths;
pub mod probe;
pub mod tools;

pub use libav::{AudioDecoder, MediaError, VideoDecoder};
