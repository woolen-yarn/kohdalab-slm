//! Raster frames: continuous phase and device codes are separate types.
//!
//! # Raster bounds
//!
//! Every path that allocates a full raster from a size it did not choose itself
//! (profiles in config or experiment JSON, direct CSV reads, pattern generation,
//! [`CodeFrame::new`]/[`CodeFrame::try_uniform`]) first calls
//! [`validate_raster`]. A raster must be at least 1 × 1, at most
//! [`MAX_RASTER_DIMENSION`] px on either side and at most [`MAX_RASTER_PIXELS`]
//! px in total. The bound leaves ample room for the Santec SLM-200
//! (1920 × 1200 = 2.3 Mpx) and 4K-class panels, while keeping the largest
//! generated frame (8-byte phase + 2-byte code per pixel) near 160 MiB instead
//! of an unbounded allocation driven by a malformed file.

use sha2::{Digest, Sha256};

/// Largest accepted raster width or height [px].
pub const MAX_RASTER_DIMENSION: u32 = 8192;
/// Largest accepted raster area [px] (16 Mpx, e.g. 4096 × 4096).
pub const MAX_RASTER_PIXELS: u64 = 1 << 24;

/// Check a raster size against the documented bounds and return its pixel
/// count. See the [module documentation](self) for the rationale.
pub fn validate_raster(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 {
        return Err("Raster size must be positive".into());
    }
    if width > MAX_RASTER_DIMENSION || height > MAX_RASTER_DIMENSION {
        return Err(format!(
            "Raster {width}×{height} exceeds the {MAX_RASTER_DIMENSION} px per-side limit"
        ));
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_RASTER_PIXELS {
        return Err(format!(
            "Raster {width}×{height} has {pixels} px; the limit is {MAX_RASTER_PIXELS} px"
        ));
    }
    usize::try_from(pixels).map_err(|_| format!("Raster {width}×{height} is too large"))
}

/// Phase in radians, wrapped to `[0, 2π)`, row-major (`y * width + x`).
#[derive(Clone, Debug, PartialEq)]
pub struct PhaseFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f64>,
}

/// Device digital codes (Santec raw contrast level), row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u16>,
}

impl CodeFrame {
    /// Frame from row-major codes; the raster must satisfy [`validate_raster`]
    /// and `data` must hold exactly `width * height` codes.
    pub fn new(width: u32, height: u32, data: Vec<u16>) -> Result<Self, String> {
        let pixels = validate_raster(width, height)?;
        if data.len() != pixels {
            return Err(format!(
                "Raster {width}×{height} needs {pixels} codes, got {}",
                data.len()
            ));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }
    /// Every pixel set to `code`, after checking the raster with
    /// [`validate_raster`] (nothing is allocated for a rejected size).
    pub fn try_uniform(width: u32, height: u32, code: u16) -> Result<Self, String> {
        let pixels = validate_raster(width, height)?;
        Ok(Self {
            width,
            height,
            data: vec![code; pixels],
        })
    }
    /// Infallible form of [`try_uniform`](Self::try_uniform) for sizes that are
    /// already validated (e.g. a validated profile raster).
    ///
    /// # Panics
    /// If the raster is outside the [`validate_raster`] bounds, instead of
    /// attempting an unbounded allocation.
    pub fn uniform(width: u32, height: u32, code: u16) -> Self {
        Self::try_uniform(width, height, code).unwrap_or_else(|e| panic!("{e}"))
    }
    pub fn get(&self, x: u32, y: u32) -> u16 {
        self.data[y as usize * self.width as usize + x as usize]
    }
    pub fn max_code(&self) -> u16 {
        self.data.iter().copied().max().unwrap_or(0)
    }
    /// SHA-256 over width, height and little-endian codes; identifies what was
    /// submitted to a backend independently of file formatting.
    pub fn digest(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(self.width.to_le_bytes());
        hash.update(self.height.to_le_bytes());
        for code in &self.data {
            hash.update(code.to_le_bytes());
        }
        hex(&hash.finalize())
    }
    /// Uniform code, if every pixel has the same value.
    pub fn uniform_code(&self) -> Option<u16> {
        let first = *self.data.first()?;
        self.data.iter().all(|&c| c == first).then_some(first)
    }
}

/// Output of rendering a pattern specification.
#[derive(Clone, Debug)]
pub enum Rendered {
    /// Raw codes chosen directly (uniform contrast, imported CSV).
    Code(CodeFrame),
    /// Generated phase with the calibrated codes derived from it.
    Phase { phase: PhaseFrame, code: CodeFrame },
}

impl Rendered {
    pub fn code(&self) -> &CodeFrame {
        match self {
            Rendered::Code(code) | Rendered::Phase { code, .. } => code,
        }
    }
    pub fn into_code(self) -> CodeFrame {
        match self {
            Rendered::Code(code) | Rendered::Phase { code, .. } => code,
        }
    }
    pub fn phase(&self) -> Option<&PhaseFrame> {
        match self {
            Rendered::Code(_) => None,
            Rendered::Phase { phase, .. } => Some(phase),
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_bounds() {
        // Santec SLM-200 hardware raster and the exact limits are accepted.
        assert_eq!(validate_raster(1920, 1200), Ok(1920 * 1200));
        assert_eq!(validate_raster(1, 1), Ok(1));
        assert_eq!(validate_raster(4096, 4096), Ok(1 << 24));
        assert_eq!(validate_raster(MAX_RASTER_DIMENSION, 2048), Ok(1 << 24));
        assert_eq!(validate_raster(1, MAX_RASTER_DIMENSION), Ok(8192));
        // Zero, one past the side limit, one past the area limit, and u32::MAX.
        for (w, h) in [
            (0, 1200),
            (1920, 0),
            (MAX_RASTER_DIMENSION + 1, 1),
            (1, MAX_RASTER_DIMENSION + 1),
            (4097, 4096),
            (MAX_RASTER_DIMENSION, 2049),
            (u32::MAX, u32::MAX),
        ] {
            assert!(validate_raster(w, h).is_err(), "{w}×{h} accepted");
        }
    }

    #[test]
    fn checked_constructors() {
        assert_eq!(
            CodeFrame::try_uniform(3, 2, 7).unwrap(),
            CodeFrame::uniform(3, 2, 7)
        );
        assert!(CodeFrame::try_uniform(u32::MAX, u32::MAX, 0).is_err());
        assert!(CodeFrame::try_uniform(0, 1, 0).is_err());
        let frame = CodeFrame::new(2, 1, vec![1, 2]).unwrap();
        assert_eq!(frame.get(1, 0), 2);
        assert!(CodeFrame::new(2, 1, vec![1]).is_err());
        assert!(CodeFrame::new(0, 0, Vec::new()).is_err());
        assert!(CodeFrame::new(MAX_RASTER_DIMENSION + 1, 1, Vec::new()).is_err());
    }

    #[test]
    #[should_panic(expected = "per-side limit")]
    fn uniform_refuses_unbounded_raster() {
        let _ = CodeFrame::uniform(u32::MAX, 1, 0);
    }
}
