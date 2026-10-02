//! AAC temporal noise shaping side-information parser.
pub use super::aac_tns::{TnsData, TnsFilter};
use super::{Result, invalid};
include!("aac_tns_impl.rs");
