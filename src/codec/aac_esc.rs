//! Owned ESC to logical ER AAC/ELD reconstruction.
use super::{aac_ep::RecoveredFrame, bits::BitReader, config::AacConfig};
use crate::{Result, invalid, unsupported};
use fvid_media::owned_aac::{
    Result as SbrResult, aac_eld_sbr::Decoder as SbrDecoder, aac_sbr_data::Data as SbrData,
    aac_sbr_header::Header as SbrHeader, bits::BitReader as SbrBitReader,
};
include!("../../crates/fvid-media/src/owned_aac/aac_esc_impl.rs");
