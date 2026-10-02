//! AAC band scalefactor syntax. Values retain their distinct spectral,
//! noise-energy and intensity-position units until reconstruction.
use super::{Result, invalid};

include!("aac_scalefactors_impl.rs");
