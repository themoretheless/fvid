//! Colour science for the decode → display path: primaries, matrices, transfer
//! functions, tone mapping, camera log curves and LUTs.
//!
//! Every curve here is implemented from its published standard rather than
//! fitted, so a code value can be checked against the standard's own tables.

pub mod lut;
pub mod primaries;
pub mod tonemap;
pub mod transfer;

pub use lut::{CubePlan, Interpolation, Lut, Lut1d, Lut3d};
pub use primaries::{
    apply, Chromaticity, MatrixCoeff, Primaries, YuvMatrix, rgb_to_rgb,
};
pub use tonemap::{ContentLight, DisplayTarget, ToneMap, curve, tone_map_rgb};
pub use transfer::{Transfer, hlg_ootf_rgb, hlg_system_gamma};
