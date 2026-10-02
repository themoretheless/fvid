//! Decoder configuration records and length-delimited NAL units.
//! These parsers extract codec initialization data; they do not decode samples.
use super::bits::BitReader;
use super::hevc_nal;
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_codec_config_impl.rs");
include!("../../crates/fvid-media/src/owned_aac/config_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    const AVC: &[u8] = &[1, 66, 0, 10, 255, 225, 0, 2, 103, 128, 1, 0, 2, 104, 128];
    #[test]
    fn avc_config_and_every_truncation() {
        let c = AvcConfig::parse(AVC).unwrap();
        assert_eq!(c.length_size, 4);
        assert_eq!(c.sps, vec![&[103, 128][..]]);
        assert_eq!(c.pps, vec![&[104, 128][..]]);
        for n in 0..AVC.len() {
            assert!(AvcConfig::parse(&AVC[..n]).is_err());
        }
        let mut bad = AVC.to_vec();
        bad[4] = 254;
        assert!(AvcConfig::parse(&bad).is_err());
        bad = AVC.to_vec();
        bad[8] = 104;
        assert!(AvcConfig::parse(&bad).is_err());
    }
    #[test]
    fn hevc_arrays_and_reserved_bits() {
        let mut h = vec![0; 23];
        h[0] = 1;
        h[1] = 1;
        h[12] = 90;
        h[13] = 240;
        h[15] = 252;
        h[16] = 253;
        h[17] = 248;
        h[18] = 248;
        h[21] = 3;
        h[22] = 1;
        h.extend_from_slice(&[0xa0, 0, 1, 0, 3, 64, 1, 128]);
        let c = HevcConfig::parse(&h).unwrap();
        assert_eq!(c.arrays[0].nal_type, 32);
        assert_eq!(c.length_size, 4);
        assert_eq!(c.bit_depth_luma, 8);
        assert_eq!(c.chroma_format, 1);
        for n in 0..h.len() {
            assert!(HevcConfig::parse(&h[..n]).is_err());
        }
        h[30] = 0; // payload is not validated by a configuration parser.
        h[29] = 0;
        assert!(HevcConfig::parse(&h).is_err());
    }
    #[test]
    fn nal_framing_terminates_on_error() {
        for size in [1, 2, 4] {
            let mut packet = vec![0; size as usize - 1];
            packet.extend_from_slice(&[2, 0x65, 128]);
            let mut units = NalUnits::new(&packet, size).unwrap();
            assert_eq!(units.next().unwrap().unwrap(), &[0x65, 128]);
            assert!(units.next().is_none());
            packet.pop();
            let mut units = NalUnits::new(&packet, size).unwrap();
            assert!(units.next().unwrap().is_err());
            assert!(units.next().is_none());
        }
        assert!(NalUnits::new(&[], 3).is_err());
    }
    #[test]
    fn aac_lc_and_sbr_absent_extension() {
        let c = AacConfig::parse(&[0x12, 0x10]).unwrap();
        assert_eq!(
            (c.sample_rate, c.channels, c.frame_samples),
            (44100, 2, 1024)
        );
        // 48 kHz mono LC + syncExtensionType 0x2b7, AOT 5, sbrPresentFlag=0.
        let c = AacConfig::parse(&[0x11, 0x88, 0x56, 0xe5, 0]).unwrap();
        assert_eq!((c.sample_rate, c.channels), (48000, 1));
        for bytes in [
            &[][..],
            &[0x12],
            &[0x12, 0],
            &[0x12, 0x11],
            &[0x11, 0x88, 0x56, 0xe5, 0x80],
        ] {
            assert!(AacConfig::parse(bytes).is_err());
        }
    }
    #[test]
    fn esds_accepts_audio_with_a_cleared_reserved_bit() {
        let esds = [
            0, 0, 0, 0, 3, 0x80, 0x80, 0x80, 0x22, 0, 0, 0, 4, 0x80, 0x80, 0x80, 0x14, 0x40, 0x14,
            0, 0x18, 0, 0, 0, 0xfa, 0, 0, 0, 0xfa, 0, 5, 0x80, 0x80, 0x80, 2, 0x12, 0x10, 6, 0x80,
            0x80, 0x80, 1, 2,
        ];
        assert_eq!(aac_specific_config(&esds).unwrap(), &[0x12, 0x10]);
        crate::codec::aac_decoder::AacDecoder::new(&esds, 44100, 2).unwrap();
        let mut invalid = esds;
        invalid[18] = 0x10; // Visual stream type, even with an otherwise valid ASC.
        assert!(aac_specific_config(&invalid).is_err());
        invalid = esds;
        invalid[17] = 0x6b; // MPEG-1 Audio, not AAC.
        assert!(aac_specific_config(&invalid).is_err());
        for end in 0..esds.len() {
            assert!(aac_specific_config(&esds[..end]).is_err());
        }
    }

    #[test]
    fn esds_extracts_only_nested_decoder_specific_data() {
        let dc = [
            0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 2, 0x12, 0x10,
        ];
        let mut esds = vec![0, 0, 0, 0, 3, 22, 0, 1, 0, 4, 17];
        esds.extend_from_slice(&dc);
        assert_eq!(aac_specific_config(&esds).unwrap(), &[0x12, 0x10]);
        for n in 0..esds.len() {
            assert!(aac_specific_config(&esds[..n]).is_err());
        }
        esds[11] = 0x6b;
        assert!(aac_specific_config(&esds).is_err());
    }
}
