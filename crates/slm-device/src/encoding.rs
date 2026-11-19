//! Santec SLM-200 DVI/HDMI 10-bit code → 24-bit RGB packing.
//!
//! 60 Hz follows SLM-200 manual v14.0 Table 6 (p. 58), as read from the
//! official PDF's search cache by the Codex session: for code `c`,
//! `R = (c >> 7) << 5`, `G = ((c >> 4) & 7) << 5`, `B = (c & 15) << 4`.
//!
//! 120 Hz (Table 7) is **not supported**: the public text extraction does not
//! fix every bit of the second frame, so it is refused rather than guessed.
//! A mode is only sent after the user confirms it on the device
//! (`ModeConfirmation`).

use serde::{Deserialize, Serialize};
use slm_core::CodeFrame;

pub const ENCODING_PROVENANCE: &str = "60 Hz: Santec SLM-200 manual v14.0 Table 6 (p. 58), R=(c>>7)<<5, G=((c>>4)&7)<<5, B=(c&15)<<4. 120 Hz (Table 7): not supported, second-frame bits unconfirmed.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputMode {
    /// 60 Hz input; one 10-bit code per pixel (Table 6).
    #[serde(rename = "60hz")]
    Hz60,
    /// 120 Hz input (Table 7). Recognized so a config naming it is refused
    /// explicitly; never encoded.
    #[serde(rename = "120hz")]
    Hz120,
}

impl InputMode {
    pub const ALL: [InputMode; 2] = [InputMode::Hz60, InputMode::Hz120];
    pub fn label(self) -> &'static str {
        match self {
            InputMode::Hz60 => "60 Hz (Table 6)",
            InputMode::Hz120 => "120 Hz (Table 7, not supported)",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            InputMode::Hz60 => "60 Hz",
            InputMode::Hz120 => "120 Hz",
        }
    }
    /// Whether this build can encode the mode.
    pub fn supported(self) -> bool {
        self == InputMode::Hz60
    }
    pub fn unsupported_reason(self) -> Option<&'static str> {
        (!self.supported()).then_some(
            "120 Hz input (Table 7) is not supported: the second-frame bit layout is not confirmed. Use 60 Hz.",
        )
    }
    /// Video refresh the Windows output must run at for this mode [Hz].
    pub fn video_refresh_hz(self) -> f64 {
        60.
    }
}

/// `(r, g, b)` for one 60 Hz pixel (Table 6).
pub fn pack60(code: u16) -> [u8; 3] {
    [
        ((code >> 7) << 5) as u8,
        (((code >> 4) & 7) << 5) as u8,
        ((code & 15) << 4) as u8,
    ]
}

/// Inverse of [`pack60`]; `None` if any bit outside the Table 6 layout is set.
pub fn unpack60(rgb: [u8; 3]) -> Option<u16> {
    if rgb[0] & 0x1F != 0 || rgb[1] & 0x1F != 0 || rgb[2] & 0x0F != 0 {
        return None;
    }
    Some((u16::from(rgb[0] >> 5) << 7) | (u16::from(rgb[1] >> 5) << 4) | u16::from(rgb[2] >> 4))
}

/// Top-down 32-bit BGRX buffer (Windows `BI_RGB` DIB memory order).
pub fn encode_bgrx(frame: &CodeFrame, mode: InputMode, max_code: u16) -> Result<Vec<u8>, String> {
    if let Some(reason) = mode.unsupported_reason() {
        return Err(reason.into());
    }
    if max_code > 1023 {
        return Err("Only 10-bit codes can be packed into RGB".into());
    }
    let mut out = Vec::with_capacity(frame.data.len() * 4);
    for (index, &code) in frame.data.iter().enumerate() {
        if code > max_code {
            let x = index % frame.width as usize;
            let y = index / frame.width as usize;
            return Err(format!("Code {code} at ({x}, {y}) is outside 0–{max_code}"));
        }
        let [r, g, b] = pack60(code);
        out.extend_from_slice(&[b, g, r, 0]);
    }
    Ok(out)
}

/// Decode a BGRX buffer back to codes (demo self-check).
pub fn decode_bgrx(
    buffer: &[u8],
    width: u32,
    height: u32,
    mode: InputMode,
) -> Result<CodeFrame, String> {
    if let Some(reason) = mode.unsupported_reason() {
        return Err(reason.into());
    }
    if buffer.len() != width as usize * height as usize * 4 {
        return Err("Encoded buffer size does not match the raster".into());
    }
    let mut data = Vec::with_capacity(buffer.len() / 4);
    for px in buffer.as_chunks::<4>().0 {
        data.push(
            unpack60([px[2], px[1], px[0]]).ok_or("Pixel has bits outside the Table 6 layout")?,
        );
    }
    Ok(CodeFrame {
        width,
        height,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table6_examples() {
        assert_eq!(pack60(0), [0, 0, 0]);
        assert_eq!(pack60(1), [0, 0, 16]);
        assert_eq!(pack60(511), [96, 224, 240]);
        assert_eq!(pack60(512), [128, 0, 0]);
        assert_eq!(pack60(1023), [224, 224, 240]);
        for code in 0..=1023 {
            assert_eq!(unpack60(pack60(code)), Some(code));
        }
        assert_eq!(unpack60([1, 0, 0]), None);
    }

    #[test]
    fn hundred_twenty_hz_is_refused() {
        let frame = CodeFrame::uniform(2, 2, 0);
        let error = encode_bgrx(&frame, InputMode::Hz120, 1023).unwrap_err();
        assert!(error.contains("not supported"), "{error}");
        assert!(!InputMode::Hz120.supported());
    }

    #[test]
    fn buffer_round_trip_and_range() {
        let frame = CodeFrame {
            width: 2,
            height: 2,
            data: vec![0, 1, 512, 1023],
        };
        let buf = encode_bgrx(&frame, InputMode::Hz60, 1023).unwrap();
        assert_eq!(&buf[4..8], &[16, 0, 0, 0]); // BGRX of code 1
        assert_eq!(decode_bgrx(&buf, 2, 2, InputMode::Hz60).unwrap(), frame);
        let bad = CodeFrame::uniform(1, 1, 1024);
        assert!(encode_bgrx(&bad, InputMode::Hz60, 1023).is_err());
    }
}
