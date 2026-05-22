//! Header parsers for PNG, JPEG, and GIF — enough to know dimensions and
//! format without decoding pixel data.

use serde::{Deserialize, Serialize};

use crate::errors::MediaError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub byte_len: u64,
}

impl Image {
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Self, MediaError> {
        let bytes = std::fs::read(&path)
            .map_err(|e| MediaError::Io(format!("read {}: {e}", path.as_ref().display())))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MediaError> {
        if bytes.len() < 4 {
            return Err(MediaError::NotEnoughBytes {
                need: 4,
                got: bytes.len(),
            });
        }
        if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            return parse_png(bytes);
        }
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return parse_jpeg(bytes);
        }
        if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            return parse_gif(bytes);
        }
        Err(MediaError::BadMagic { kind: "image" })
    }
}

fn parse_png(bytes: &[u8]) -> Result<Image, MediaError> {
    // Layout: 8-byte signature, then IHDR chunk:
    //   bytes  8..12 — chunk length (must be 13)
    //   bytes 12..16 — chunk type "IHDR"
    //   bytes 16..20 — width  (BE u32)
    //   bytes 20..24 — height (BE u32)
    if bytes.len() < 24 {
        return Err(MediaError::Truncated {
            context: "PNG IHDR",
        });
    }
    if &bytes[12..16] != b"IHDR" {
        return Err(MediaError::BadMagic { kind: "PNG IHDR" });
    }
    let width = read_u32_be(&bytes[16..20]);
    let height = read_u32_be(&bytes[20..24]);
    Ok(Image {
        format: ImageFormat::Png,
        width,
        height,
        byte_len: bytes.len() as u64,
    })
}

fn parse_jpeg(bytes: &[u8]) -> Result<Image, MediaError> {
    // Walk segments looking for a Start-Of-Frame marker (SOF0..SOF15, except
    // SOF4=DHT, SOF8=JPG, SOF12=DAC). Each segment is `FF Mn LLLL ...` where
    // `LLLL` is BE u16 length *including* itself.
    let mut i = 2; // skip SOI (FF D8)
    while i + 1 < bytes.len() {
        if bytes[i] != 0xFF {
            return Err(MediaError::Truncated {
                context: "JPEG marker stream",
            });
        }
        // Skip fill bytes.
        let mut marker = bytes[i + 1];
        let mut p = i + 2;
        while marker == 0xFF && p < bytes.len() {
            marker = bytes[p];
            p += 1;
        }
        let mn = marker;
        let payload_start = p;
        match mn {
            0xD0..=0xD7 | 0xD8 | 0xD9 => {
                // RSTn / SOI / EOI — no length
                i = payload_start;
                if mn == 0xD9 {
                    break;
                }
                continue;
            }
            0xC0..=0xCF if !matches!(mn, 0xC4 | 0xC8 | 0xCC) => {
                // SOFn: payload is `LLLL P H_HI H_LO W_HI W_LO ...`
                if payload_start + 7 > bytes.len() {
                    return Err(MediaError::Truncated {
                        context: "JPEG SOF",
                    });
                }
                let h = ((bytes[payload_start + 3] as u32) << 8) | (bytes[payload_start + 4] as u32);
                let w = ((bytes[payload_start + 5] as u32) << 8) | (bytes[payload_start + 6] as u32);
                return Ok(Image {
                    format: ImageFormat::Jpeg,
                    width: w,
                    height: h,
                    byte_len: bytes.len() as u64,
                });
            }
            _ => {
                if payload_start + 2 > bytes.len() {
                    return Err(MediaError::Truncated {
                        context: "JPEG segment length",
                    });
                }
                let len = ((bytes[payload_start] as usize) << 8) | (bytes[payload_start + 1] as usize);
                if len < 2 {
                    return Err(MediaError::Truncated {
                        context: "JPEG segment length",
                    });
                }
                i = payload_start + len;
            }
        }
    }
    Err(MediaError::Truncated {
        context: "JPEG (no SOF found)",
    })
}

fn parse_gif(bytes: &[u8]) -> Result<Image, MediaError> {
    if bytes.len() < 10 {
        return Err(MediaError::Truncated { context: "GIF LSD" });
    }
    let width = (bytes[6] as u32) | ((bytes[7] as u32) << 8);
    let height = (bytes[8] as u32) | ((bytes[9] as u32) << 8);
    Ok(Image {
        format: ImageFormat::Gif,
        width,
        height,
        byte_len: bytes.len() as u64,
    })
}

fn read_u32_be(b: &[u8]) -> u32 {
    debug_assert!(b.len() >= 4);
    ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-craft the minimum bytes to look like a PNG with given dims.
    fn fake_png(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend(&13u32.to_be_bytes());
        v.extend(b"IHDR");
        v.extend(&w.to_be_bytes());
        v.extend(&h.to_be_bytes());
        // depth + color + compression + filter + interlace + CRC (8 bytes)
        v.extend(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        v
    }

    /// Minimal JPEG: SOI + APP0 stub + SOF0 with dims + EOI.
    fn fake_jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        // APP0 marker
        v.extend(&[0xFF, 0xE0]);
        v.extend(&16u16.to_be_bytes());
        v.extend(b"JFIF\0");
        v.extend(&[1, 2, 0, 0, 1, 0, 1, 0, 0]);
        // SOF0
        v.extend(&[0xFF, 0xC0]);
        v.extend(&17u16.to_be_bytes());
        v.push(8); // precision
        v.extend(&h.to_be_bytes());
        v.extend(&w.to_be_bytes());
        v.extend(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        v.extend(&[0xFF, 0xD9]);
        v
    }

    fn fake_gif(w: u16, h: u16) -> Vec<u8> {
        let mut v = b"GIF89a".to_vec();
        v.extend(&w.to_le_bytes());
        v.extend(&h.to_le_bytes());
        v.push(0); // packed
        v.push(0); // background color
        v.push(0); // pixel aspect
        v
    }

    #[test]
    fn png_dims_round_trip() {
        let img = Image::from_bytes(&fake_png(640, 480)).unwrap();
        assert_eq!(
            img,
            Image {
                format: ImageFormat::Png,
                width: 640,
                height: 480,
                byte_len: fake_png(640, 480).len() as u64,
            }
        );
    }

    #[test]
    fn jpeg_dims_round_trip() {
        let img = Image::from_bytes(&fake_jpeg(1280, 720)).unwrap();
        assert_eq!(img.format, ImageFormat::Jpeg);
        assert_eq!(img.width, 1280);
        assert_eq!(img.height, 720);
    }

    #[test]
    fn gif_dims_round_trip() {
        let img = Image::from_bytes(&fake_gif(100, 50)).unwrap();
        assert_eq!(img.format, ImageFormat::Gif);
        assert_eq!(img.width, 100);
        assert_eq!(img.height, 50);
    }

    #[test]
    fn bad_magic_rejected() {
        let err = Image::from_bytes(b"NOTANIMAGE").unwrap_err();
        assert!(matches!(err, MediaError::BadMagic { .. }));
    }

    #[test]
    fn truncated_png_errors() {
        let err = Image::from_bytes(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]).unwrap_err();
        assert!(matches!(err, MediaError::Truncated { .. }));
    }
}
