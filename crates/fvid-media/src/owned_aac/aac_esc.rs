//! Exact ESC instances to logical ELD payload; ISO 4.5.2.4 and table 4.148.
use super::{
    Result as SbrResult, aac_eld_sbr::Decoder as SbrDecoder, aac_sbr_data::Data as SbrData,
    aac_sbr_header::Header as SbrHeader, bits::BitReader as SbrBitReader,
};
use super::{Result, aac_ep::RecoveredFrame, bits::BitReader, config::AacConfig, invalid};
include!("aac_esc_impl.rs");
