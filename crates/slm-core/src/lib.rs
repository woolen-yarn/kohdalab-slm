//! Pattern generation, one-axis sweeps, experiment files, Santec CSV and PNG
//! export for KohdaLab SLM.
//!
//! Continuous phase (`PhaseFrame`, radians) and device digital codes
//! (`CodeFrame`, Santec raw `u16`) are separate types on purpose: a raw code is
//! never produced by accident from an uncalibrated phase, and imported CSV
//! frames are sent as stored, without rotation or calibration.

#![forbid(unsafe_code)]

pub mod csv;
pub mod experiment;
pub mod frame;
pub mod heatmap;
pub mod pattern;
pub mod sweep;
pub mod units;

pub use frame::{CodeFrame, PhaseFrame, Rendered};
pub use pattern::{
    GenerationProfile, Geometry, PatternSpec, PhaseCalibration, PhaseMapping, Raster,
    WaveComponent, Waveform,
};
pub use units::{AngleUnit, AngleValue, Optics, PeriodUnit, PeriodValue, PhaseUnit, PhaseValue};
