//! Owned AAC coupling syntax and dependent spectral mixing.
use super::Error;
use super::aac_coupling as coupling_mix;
use super::aac_geometry::BandTables;
use super::{Result, invalid};
include!("aac_coupling_impl.rs");
