//! Colour science for the decode → display path: primaries, matrices, transfer
//! functions, tone mapping, camera log curves and LUTs.
//!
//! Every curve here is implemented from its published standard rather than
//! fitted, so a code value can be checked against the standard's own tables.

pub mod grade;
pub mod hdr;
pub mod log;
pub mod lut;
pub mod primaries;
pub mod tonemap;
pub mod transfer;

pub use grade::{Grade, Settings};
pub use hdr::{
    mdcv_payload, ColourDescription, HdrMetadata, MasteringDisplay, CLLI_PAYLOAD_LEN,
    MDCV_PAYLOAD_LEN, SEI_CLLI, SEI_MDCV,
};
pub use log::{Codes, Log};
pub use lut::{CubePlan, Interpolation, Lut, Lut1d, Lut3d};
pub use primaries::{apply, rgb_to_rgb, Chromaticity, MatrixCoeff, Primaries, YuvMatrix};
pub use tonemap::{curve, tone_map_rgb, ContentLight, DisplayTarget, ToneMap};
pub use transfer::{hlg_ootf_rgb, hlg_system_gamma, Transfer};
