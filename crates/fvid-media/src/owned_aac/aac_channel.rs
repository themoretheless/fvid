//! Owned AAC-LC individual channel parsing.
use super::Error;
use super::aac_geometry::BandTables;
use super::aac_tns_syntax as tns_syntax;
use super::{Result, invalid, unsupported};
include!("aac_channel_impl.rs");
