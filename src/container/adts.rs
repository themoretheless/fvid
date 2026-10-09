//! Compatibility entrypoint for the owned media-library ADTS parser.
use crate::codec::{config::AacConfig, bits::BitReader, aac_pce::{ProgramConfig, skip_data_stream, skip_fill}};
use crate::{Result, invalid};
use fvid_media::owned_aac::adts_crc;
include!("../../crates/fvid-media/src/owned_aac/adts_impl.rs");

#[cfg(test)]
mod mp4_descriptor_tests {
    #[test]
    fn sl_configuration_is_inside_multibyte_es_descriptor() {
        for width in [2, 107, 108, 127, 128, 255, 4096] {
            let mut asc = vec![0; width];
            asc[..2].copy_from_slice(&[0x11, 0x90]);
            let descriptor = super::esds_for_mp4(&asc).unwrap();
            assert_eq!(crate::codec::config::aac_specific_config(&descriptor).unwrap(), asc);
            let mut length = 0usize;
            let mut cursor = 5;
            loop {
                let byte = descriptor[cursor];
                cursor += 1;
                length = (length << 7) | usize::from(byte & 127);
                if byte & 128 == 0 {break;}
            }
            assert_eq!(cursor + length, descriptor.len());
            assert_eq!(&descriptor[descriptor.len()-3..], &[6, 1, 2]);
        }
    }
}
