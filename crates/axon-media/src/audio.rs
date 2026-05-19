//! WAV header parser.
//!
//! Reads the `fmt ` and `data` sub-chunks of a RIFF/WAVE file to recover
//! sample rate, channel count, bit depth, and duration. Doesn't decode the
//! PCM samples themselves.

use serde::{Deserialize, Serialize};

use crate::errors::MediaError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioFormat {
    Wav,
}

impl AudioFormat {
    pub fn mime(self) -> &'static str {
        match self {
            AudioFormat::Wav => "audio/wav",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Audio {
    pub format: AudioFormat,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    /// Duration in milliseconds. Zero if the `data` chunk wasn't found.
    pub duration_ms: u32,
    pub byte_len: u64,
}

impl Audio {
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Self, MediaError> {
        let bytes = std::fs::read(&path)
            .map_err(|e| MediaError::Io(format!("read {}: {e}", path.as_ref().display())))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MediaError> {
        if bytes.len() < 12 {
            return Err(MediaError::NotEnoughBytes {
                need: 12,
                got: bytes.len(),
            });
        }
        if &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(MediaError::BadMagic { kind: "WAV/RIFF" });
        }
        // Walk sub-chunks.
        let mut i = 12;
        let mut fmt: Option<(u16, u32, u16, u32)> = None;
        let mut data_len: Option<u32> = None;
        while i + 8 <= bytes.len() {
            let id = &bytes[i..i + 4];
            let size = read_u32_le(&bytes[i + 4..i + 8]) as usize;
            let payload_start = i + 8;
            let payload_end = payload_start + size;
            if payload_end > bytes.len() {
                return Err(MediaError::Truncated {
                    context: "WAV sub-chunk",
                });
            }
            match id {
                b"fmt " => {
                    if size < 16 {
                        return Err(MediaError::Truncated {
                            context: "WAV fmt chunk",
                        });
                    }
                    let audio_format = read_u16_le(&bytes[payload_start..payload_start + 2]);
                    let channels = read_u16_le(&bytes[payload_start + 2..payload_start + 4]);
                    let sample_rate = read_u32_le(&bytes[payload_start + 4..payload_start + 8]);
                    let byte_rate = read_u32_le(&bytes[payload_start + 8..payload_start + 12]);
                    let bps = read_u16_le(&bytes[payload_start + 14..payload_start + 16]);
                    if audio_format != 1 && audio_format != 3 && audio_format != 0xFFFE {
                        return Err(MediaError::Unsupported {
                            detail: format!("WAV audio format tag 0x{audio_format:04x}"),
                        });
                    }
                    fmt = Some((channels, sample_rate, bps, byte_rate));
                }
                b"data" => {
                    data_len = Some(size as u32);
                }
                _ => {}
            }
            // Chunks are padded to even byte boundaries.
            let advance = if size & 1 == 1 { size + 1 } else { size };
            i = payload_start + advance;
            if i >= bytes.len() {
                break;
            }
        }
        let (channels, sample_rate, bps, byte_rate) = fmt.ok_or(MediaError::Truncated {
            context: "WAV fmt chunk (missing)",
        })?;
        let duration_ms = match (data_len, byte_rate) {
            (Some(dl), br) if br > 0 => ((dl as u64 * 1000) / br as u64) as u32,
            _ => 0,
        };
        Ok(Audio {
            format: AudioFormat::Wav,
            sample_rate_hz: sample_rate,
            channels,
            bits_per_sample: bps,
            duration_ms,
            byte_len: bytes.len() as u64,
        })
    }
}

fn read_u16_le(b: &[u8]) -> u16 {
    (b[0] as u16) | ((b[1] as u16) << 8)
}

fn read_u32_le(b: &[u8]) -> u32 {
    (b[0] as u32) | ((b[1] as u32) << 8) | ((b[2] as u32) << 16) | ((b[3] as u32) << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_wav(sample_rate: u32, channels: u16, bps: u16, samples_per_channel: u32) -> Vec<u8> {
        let byte_rate = sample_rate * channels as u32 * (bps / 8) as u32;
        let block_align = channels * bps / 8;
        let data_bytes = samples_per_channel * channels as u32 * (bps / 8) as u32;

        let mut v = b"RIFF".to_vec();
        v.extend(&(36u32 + data_bytes).to_le_bytes());
        v.extend(b"WAVE");

        // fmt sub-chunk
        v.extend(b"fmt ");
        v.extend(&16u32.to_le_bytes());
        v.extend(&1u16.to_le_bytes()); // PCM
        v.extend(&channels.to_le_bytes());
        v.extend(&sample_rate.to_le_bytes());
        v.extend(&byte_rate.to_le_bytes());
        v.extend(&block_align.to_le_bytes());
        v.extend(&bps.to_le_bytes());

        // data sub-chunk
        v.extend(b"data");
        v.extend(&data_bytes.to_le_bytes());
        v.extend(std::iter::repeat(0u8).take(data_bytes as usize));
        v
    }

    #[test]
    fn pcm_44100_stereo_16bit_parses() {
        let bytes = fake_wav(44100, 2, 16, 44100); // 1 second
        let a = Audio::from_bytes(&bytes).unwrap();
        assert_eq!(a.sample_rate_hz, 44100);
        assert_eq!(a.channels, 2);
        assert_eq!(a.bits_per_sample, 16);
        assert_eq!(a.duration_ms, 1000);
    }

    #[test]
    fn pcm_8khz_mono_8bit_half_second_parses() {
        let bytes = fake_wav(8000, 1, 8, 4000); // 0.5 s
        let a = Audio::from_bytes(&bytes).unwrap();
        assert_eq!(a.sample_rate_hz, 8000);
        assert_eq!(a.duration_ms, 500);
    }

    #[test]
    fn missing_fmt_chunk_errors() {
        let mut v = b"RIFF\x00\x00\x00\x00WAVE".to_vec();
        // data only, no fmt
        v.extend(b"data");
        v.extend(&0u32.to_le_bytes());
        let err = Audio::from_bytes(&v).unwrap_err();
        assert!(matches!(err, MediaError::Truncated { .. }));
    }

    #[test]
    fn bad_magic_errors() {
        let err = Audio::from_bytes(b"NOTAWAVEFILE").unwrap_err();
        assert!(matches!(err, MediaError::BadMagic { .. }));
    }
}
