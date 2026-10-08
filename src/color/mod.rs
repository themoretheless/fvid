//! Colour science for the decode → display path: primaries, matrices, transfer
//! functions, tone mapping, camera log curves and LUTs.
//!
//! Every curve here is implemented from its published standard rather than
//! fitted, so a code value can be checked against the standard's own tables.

pub mod grade;
pub use fvid_codecs::color::hdr;
pub use fvid_codecs::color::log;
pub mod lut;
pub use fvid_codecs::color::primaries;
pub use fvid_codecs::color::tonemap;
pub use fvid_codecs::color::transfer;
#[cfg(target_arch = "x86_64")]
pub mod simd_rgb_convert;

pub use grade::{Grade, Settings, ShaderLook, ShaderStages};
pub use hdr::{
    CLLI_PAYLOAD_LEN, ColourDescription, HdrMetadata, MDCV_PAYLOAD_LEN, MasteringDisplay, SEI_CLLI,
    SEI_MDCV, mdcv_payload,
};
pub use log::{Codes, Log};
pub use lut::{CubePlan, Interpolation, Lut, Lut1d, Lut3d};
pub use primaries::{Chromaticity, MatrixCoeff, Primaries, YuvMatrix, apply, rgb_to_rgb};
pub use tonemap::{ContentLight, DisplayTarget, ToneMap, curve, tone_map_rgb};
pub use transfer::{Transfer, hlg_ootf_rgb, hlg_system_gamma};
