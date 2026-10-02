//! AAC ordinary spectral-band inverse quantization, before window synthesis.
//! Noise and intensity codebooks require their own reconstruction operations.
use super::{Result, invalid};

include!("aac_quant_impl.rs");
