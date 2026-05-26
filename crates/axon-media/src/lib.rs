//! `axon-media` — typed multimodal primitives.
//!
//! Stage 12 ships **safe, header-only parsers**: we don't decode pixels or
//! resample audio. The runtime needs to know enough about a media object
//! (kind, dimensions, sample rate, page count) to type-check programs and
//! enforce model-capability matches at compile time; full decode lives
//! behind capability-gated tools.
//!
//! Why pure-Rust, header-only?
//!
//!   * Header parsing is bounded, side-effect-free, and easy to audit. A
//!     malformed file produces a typed error, never a decoder exploit.
//!   * No native dependencies → builds reproducibly on every platform.
//!   * Down-stream tools (`image.analyze`, `audio.transcribe`) can pull in
//!     real decoders when they need actual pixels or samples.

pub mod audio;
pub mod document;
pub mod errors;
pub mod generate;
pub mod image;
pub mod sniff;

pub use audio::Audio;
pub use document::Document;
pub use errors::MediaError;
pub use generate::{
    GenAudioFormat, GenImageFormat, GenerateAudioRequest, GenerateImageRequest, GeneratedAudio,
    GeneratedImage, MediaGenError, MediaProvider, MockProvider,
};
pub use image::{Image, ImageFormat};
pub use sniff::sniff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Audio,
    Document,
    Unknown,
}
