//! Device profiles, output-display validation, 10-bit RGB encoding and the
//! single serialized device worker for KohdaLab SLM.
//!
//! The worker is the only place that talks to an output backend. Every apply
//! re-enumerates displays and re-checks the saved output identity, so a
//! renumbered, removed, duplicated or re-moded display stops output instead of
//! sending a pattern to the wrong screen.

pub mod backend;
pub mod config;
pub mod demo;
pub mod display;
pub mod encoding;
pub mod profile;
pub mod wavelength;
#[cfg(windows)]
mod windows;
pub mod worker;

pub use backend::{BackendKind, OutputBackend, OutputTarget, ShowReceipt, Verification};
pub use config::{AppConfig, ModeConfirmation, OutputConfig};
pub use display::{BlockReason, DisplayInfo, DisplaySelection, OutputRequirements};
pub use encoding::InputMode;
pub use profile::{DeviceProfile, SlmModel};
pub use worker::{ApplyItem, Command, DeviceState, DeviceWorker, Event, FrameSource};
