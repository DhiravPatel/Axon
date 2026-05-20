//! Multimodal generation (§51.2 / §51.3).
//!
//! Stage 12 shipped *input* primitives (typed Image/Audio/Document
//! values, header-only safe parsing). Generation lands here: typed
//! `GenerateImageRequest` / `GenerateAudioRequest` / `GenerateVideoRequest`
//! descriptors plus a `MediaProvider` trait the runtime can register
//! a concrete backend against (OpenAI gpt-image-1, Stability, ElevenLabs,
//! local diffusers, ...).
//!
//! What's shipped here is the **interface** + a deterministic `MockProvider`
//! that returns a fixed byte signature so the type machinery and host
//! bindings have something to round-trip against without making a
//! network call. Real provider drivers ship as separate crates that
//! plug into the trait.
//!
//! Why a trait instead of an enum of providers? Three reasons:
//!
//!   * Drivers are out-of-tree (the workspace shouldn't depend on every
//!     provider's SDK).
//!   * Drivers may want to share state (HTTP client, rate limiter, etc.)
//!     that doesn't fit a stateless enum.
//!   * The trait is the same shape the Stage 6 `ModelDriver` uses, so
//!     `axon login` / `with budget` / `axon trace` integrate uniformly.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenImageFormat {
    Png,
    Jpeg,
    Webp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenAudioFormat {
    Mp3,
    Wav,
    Flac,
    Opus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateImageRequest {
    pub prompt: String,
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_image_format")]
    pub format: GenImageFormat,
    #[serde(default)]
    pub negative_prompt: String,
    #[serde(default)]
    pub seed: u64,
    /// `1..=n` candidates per request.
    #[serde(default = "default_n")]
    pub n: u32,
}

fn default_image_format() -> GenImageFormat {
    GenImageFormat::Png
}
fn default_n() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateAudioRequest {
    pub prompt: String,
    /// Free-form voice id (provider-specific).
    #[serde(default)]
    pub voice: String,
    pub sample_rate: u32,
    #[serde(default = "default_audio_format")]
    pub format: GenAudioFormat,
    #[serde(default)]
    pub max_duration_secs: u32,
    #[serde(default)]
    pub seed: u64,
}

fn default_audio_format() -> GenAudioFormat {
    GenAudioFormat::Mp3
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratedImage {
    pub bytes: Vec<u8>,
    pub format: GenImageFormat,
    pub width: u32,
    pub height: u32,
    /// Provider's job-id / generation-id, when applicable.
    #[serde(default)]
    pub provider_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratedAudio {
    pub bytes: Vec<u8>,
    pub format: GenAudioFormat,
    pub sample_rate: u32,
    pub duration_ms: u32,
    #[serde(default)]
    pub provider_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaGenError {
    Unsupported(String),
    Provider(String),
    Validation(String),
}

impl std::fmt::Display for MediaGenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaGenError::Unsupported(s) => write!(f, "unsupported: {s}"),
            MediaGenError::Provider(s) => write!(f, "provider error: {s}"),
            MediaGenError::Validation(s) => write!(f, "validation error: {s}"),
        }
    }
}

impl std::error::Error for MediaGenError {}

pub trait MediaProvider {
    fn name(&self) -> &str;
    fn generate_image(
        &self,
        _req: &GenerateImageRequest,
    ) -> Result<Vec<GeneratedImage>, MediaGenError> {
        Err(MediaGenError::Unsupported(format!(
            "provider `{}` does not implement image generation",
            self.name()
        )))
    }
    fn generate_audio(
        &self,
        _req: &GenerateAudioRequest,
    ) -> Result<GeneratedAudio, MediaGenError> {
        Err(MediaGenError::Unsupported(format!(
            "provider `{}` does not implement audio generation",
            self.name()
        )))
    }
}

/// Deterministic mock — produces a tiny PNG signature (8 bytes) and a
/// short WAV header padded with silence, so the type plumbing works
/// end-to-end without a real backend. Useful in CI; useful in tests;
/// useful for offline-replay where any byte output passes equality.
pub struct MockProvider {
    pub name: String,
}

impl MockProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl MediaProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn generate_image(
        &self,
        req: &GenerateImageRequest,
    ) -> Result<Vec<GeneratedImage>, MediaGenError> {
        validate_image(req)?;
        let mut out = Vec::with_capacity(req.n as usize);
        for i in 0..req.n.max(1) {
            // 8-byte PNG signature so `axon-media`'s sniffer recognizes
            // the result as a PNG without us having to emit a valid
            // image. Real providers replace this with actual pixels.
            let bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
            out.push(GeneratedImage {
                bytes,
                format: req.format,
                width: req.width,
                height: req.height,
                provider_id: format!("mock:{}#{i}", self.name),
            });
        }
        Ok(out)
    }

    fn generate_audio(
        &self,
        req: &GenerateAudioRequest,
    ) -> Result<GeneratedAudio, MediaGenError> {
        validate_audio(req)?;
        let bytes = b"RIFF\0\0\0\x24WAVEfmt ".to_vec();
        Ok(GeneratedAudio {
            bytes,
            format: req.format,
            sample_rate: req.sample_rate,
            duration_ms: 0,
            provider_id: format!("mock:{}", self.name),
        })
    }
}

fn validate_image(req: &GenerateImageRequest) -> Result<(), MediaGenError> {
    if req.prompt.is_empty() {
        return Err(MediaGenError::Validation("prompt is empty".into()));
    }
    if req.width == 0 || req.height == 0 {
        return Err(MediaGenError::Validation(
            "width and height must be positive".into(),
        ));
    }
    if req.width > 4096 || req.height > 4096 {
        return Err(MediaGenError::Validation(
            "width and height must be <= 4096 in v0".into(),
        ));
    }
    if req.n == 0 || req.n > 8 {
        return Err(MediaGenError::Validation(
            "n must be in 1..=8 in v0".into(),
        ));
    }
    Ok(())
}

fn validate_audio(req: &GenerateAudioRequest) -> Result<(), MediaGenError> {
    if req.prompt.is_empty() {
        return Err(MediaGenError::Validation("prompt is empty".into()));
    }
    if req.sample_rate == 0 {
        return Err(MediaGenError::Validation(
            "sample_rate must be positive".into(),
        ));
    }
    if req.sample_rate < 8_000 || req.sample_rate > 192_000 {
        return Err(MediaGenError::Validation(
            "sample_rate must be in 8000..=192000".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_image_returns_png_signature() {
        let p = MockProvider::new("test");
        let req = GenerateImageRequest {
            prompt: "a friendly cat".into(),
            width: 512,
            height: 512,
            format: GenImageFormat::Png,
            negative_prompt: String::new(),
            seed: 0,
            n: 2,
        };
        let out = p.generate_image(&req).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].bytes[0..4], [0x89, b'P', b'N', b'G']);
        assert_eq!(out[0].width, 512);
    }

    #[test]
    fn empty_prompt_rejected() {
        let p = MockProvider::new("test");
        let req = GenerateImageRequest {
            prompt: String::new(),
            width: 64,
            height: 64,
            format: GenImageFormat::Png,
            negative_prompt: String::new(),
            seed: 0,
            n: 1,
        };
        let err = p.generate_image(&req).unwrap_err();
        assert!(matches!(err, MediaGenError::Validation(_)));
    }

    #[test]
    fn zero_dim_rejected() {
        let p = MockProvider::new("test");
        let req = GenerateImageRequest {
            prompt: "hi".into(),
            width: 0,
            height: 100,
            format: GenImageFormat::Png,
            negative_prompt: String::new(),
            seed: 0,
            n: 1,
        };
        assert!(p.generate_image(&req).is_err());
    }

    #[test]
    fn n_over_eight_rejected() {
        let p = MockProvider::new("test");
        let req = GenerateImageRequest {
            prompt: "hi".into(),
            width: 64,
            height: 64,
            format: GenImageFormat::Png,
            negative_prompt: String::new(),
            seed: 0,
            n: 9,
        };
        assert!(p.generate_image(&req).is_err());
    }

    #[test]
    fn audio_validation_catches_bad_sample_rate() {
        let p = MockProvider::new("test");
        let req = GenerateAudioRequest {
            prompt: "test".into(),
            voice: String::new(),
            sample_rate: 4_000,
            format: GenAudioFormat::Mp3,
            max_duration_secs: 0,
            seed: 0,
        };
        assert!(p.generate_audio(&req).is_err());
    }

    #[test]
    fn audio_round_trip_returns_riff() {
        let p = MockProvider::new("test");
        let req = GenerateAudioRequest {
            prompt: "test".into(),
            voice: "default".into(),
            sample_rate: 44_100,
            format: GenAudioFormat::Wav,
            max_duration_secs: 5,
            seed: 0,
        };
        let out = p.generate_audio(&req).unwrap();
        assert!(out.bytes.starts_with(b"RIFF"));
        assert_eq!(out.sample_rate, 44_100);
    }

    #[test]
    fn default_provider_unsupported_for_unimplemented_method() {
        struct OnlyAudio;
        impl MediaProvider for OnlyAudio {
            fn name(&self) -> &str {
                "audio-only"
            }
            fn generate_audio(
                &self,
                _: &GenerateAudioRequest,
            ) -> Result<GeneratedAudio, MediaGenError> {
                Ok(GeneratedAudio {
                    bytes: Vec::new(),
                    format: GenAudioFormat::Mp3,
                    sample_rate: 44_100,
                    duration_ms: 0,
                    provider_id: String::new(),
                })
            }
        }
        let p = OnlyAudio;
        let req = GenerateImageRequest {
            prompt: "x".into(),
            width: 1,
            height: 1,
            format: GenImageFormat::Png,
            negative_prompt: String::new(),
            seed: 0,
            n: 1,
        };
        let err = p.generate_image(&req).unwrap_err();
        assert!(matches!(err, MediaGenError::Unsupported(_)));
    }
}
