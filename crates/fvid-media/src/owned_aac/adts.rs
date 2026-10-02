//! Owned ADTS framing, CRC boundaries and PCE configuration.
use super::{Result, invalid};
use super::{config::AacConfig, bits::BitReader, aac_pce::{ProgramConfig, skip_data_stream, skip_fill}};
include!("adts_impl.rs");
