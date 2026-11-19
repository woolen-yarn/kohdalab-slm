//! Raster frames: continuous phase and device codes are separate types.

use sha2::{Digest, Sha256};

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
    pub fn uniform(width: u32, height: u32, code: u16) -> Self {
        Self {
            width,
            height,
            data: vec![code; width as usize * height as usize],
        }
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
