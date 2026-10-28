//! Owned ADTS framing, single/multiblock CRC checking and PCE configuration.
use super::{Result, invalid, adts_crc};
use super::{config::AacConfig, bits::BitReader, aac_pce::{ProgramConfig, skip_data_stream, skip_fill}};
#[cfg(test)]
use crate::owned_codec_config::aac_specific_config as esds_specific_config;
include!("adts_impl.rs");
