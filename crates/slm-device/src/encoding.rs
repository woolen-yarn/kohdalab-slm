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

/// Largest code the 60 Hz Table 6 layout can carry (10 bit).
pub const MAX_CODE_60HZ: u16 = 1023;

/// `(r, g, b)` for one 60 Hz pixel (Table 6), or `None` if `code` exceeds
/// [`MAX_CODE_60HZ`]. Without the check, bit 10 and above would be truncated
/// by the `u8` red channel and a different, valid-looking code would be sent.
pub fn try_pack60(code: u16) -> Option<[u8; 3]> {
    if code > MAX_CODE_60HZ {
        return None;
    }
    Some([
        ((code >> 7) << 5) as u8,
        (((code >> 4) & 7) << 5) as u8,
        ((code & 15) << 4) as u8,
    ])
}

/// `(r, g, b)` for one 60 Hz pixel (Table 6).
///
/// # Panics
/// If `code > MAX_CODE_60HZ`; such a code has no Table 6 representation. Use
/// [`try_pack60`] for unchecked input, or [`encode_bgrx`], which validates
/// every code and reports its position.
pub fn pack60(code: u16) -> [u8; 3] {
    try_pack60(code).unwrap_or_else(|| {
        panic!("Code {code} is outside the 60 Hz Table 6 range 0–{MAX_CODE_60HZ}")
    })
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
    if max_code > MAX_CODE_60HZ {
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
    let pixels = slm_core::frame::validate_raster(width, height)?;
    if buffer.len() != pixels * 4 {
        return Err("Encoded buffer size does not match the raster".into());
    }
    let mut data = Vec::with_capacity(pixels);
    for px in buffer.as_chunks::<4>().0 {
        data.push(
            unpack60([px[2], px[1], px[0]]).ok_or("Pixel has bits outside the Table 6 layout")?,
        );
    }
    CodeFrame::new(width, height, data)
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
    fn pack60_precondition_is_enforced() {
        assert_eq!(try_pack60(MAX_CODE_60HZ), Some([224, 224, 240]));
        // 1024 would otherwise wrap to the packing of code 0.
        assert_eq!(try_pack60(MAX_CODE_60HZ + 1), None);
        assert_eq!(try_pack60(u16::MAX), None);
        assert!(std::panic::catch_unwind(|| pack60(1024)).is_err());
        // encode_bgrx reports the offending pixel instead of panicking, even
        // when the caller's max_code would otherwise admit the code.
        let frame = CodeFrame::new(2, 1, vec![5, 1024]).unwrap();
        let error = encode_bgrx(&frame, InputMode::Hz60, 1023).unwrap_err();
        assert!(error.contains("(1, 0)"), "{error}");
        assert!(encode_bgrx(&frame, InputMode::Hz60, 1024).is_err());
    }

    #[test]
    fn decode_refuses_out_of_bounds_raster() {
        assert!(decode_bgrx(&[], 0, 0, InputMode::Hz60).is_err());
        let error = decode_bgrx(&[], u32::MAX, u32::MAX, InputMode::Hz60).unwrap_err();
        assert!(error.contains("limit"), "{error}");
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
