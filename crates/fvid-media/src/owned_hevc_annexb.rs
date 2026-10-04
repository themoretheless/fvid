//! Own NVENC HEVC Annex B to hvcC and length-prefixed Matroska packet framing.
use fvid_codecs::codec::{
    config::HevcConfig, hevc_nal::NalHeader, hevc_pps::Pps, hevc_sps::Sps, hevc_vps::Vps,
};
pub struct Packet {
    pub configuration: Option<Vec<u8>>,
    pub sample: Vec<u8>,
    pub sync: bool,
}

pub fn convert(data: &[u8], max_bytes: usize) -> Result<Packet, String> {
    if data.is_empty() || data.len() > max_bytes {
        return Err("HEVC Annex B packet is empty or exceeds byte limit".into());
    }
    let (first, prefix) =
        crate::owned_avc_annexb::start(data, 0).ok_or("HEVC Annex B start code is missing")?;
    if data[..first].iter().any(|b| *b != 0) {
        return Err("HEVC Annex B has nonzero leading bytes".into());
    }
    let mut at = first + prefix;
    let mut sets: [Vec<&[u8]>; 3] = std::array::from_fn(|_| Vec::new());
    let mut sample = Vec::new();
    let mut sync = false;
    let mut picture = false;
    loop {
        let next = crate::owned_avc_annexb::start(data, at);
        let mut end = next.map_or(data.len(), |n| n.0);
        while end > at && data[end - 1] == 0 {
            end -= 1;
        }
        let nal = &data[at..end];
        let header = NalHeader::parse(nal).map_err(|e| e.to_string())?;
        header.require_base_layer().map_err(|e| e.to_string())?;
        if !matches!(header.unit_type,0..=9|16..=21|32..=40) {
            return Err("HEVC Annex B contains reserved/unsupported NAL type".into());
        }
        if matches!(header.unit_type, 32..=34) {
            let entries = &mut sets[(header.unit_type - 32) as usize];
            if !entries.contains(&nal) {
                entries.try_reserve(1).map_err(|e| e.to_string())?;
                entries.push(nal);
            }
        }
        picture |= header.is_vcl();
        sync |= header.is_irap();
        let length = sample
            .len()
            .checked_add(4)
            .and_then(|n| n.checked_add(nal.len()))
            .filter(|n| *n <= max_bytes)
            .ok_or("HEVC converted packet exceeds byte limit")?;
        sample
            .try_reserve(length - sample.len())
            .map_err(|e| e.to_string())?;
        sample.extend_from_slice(
            &u32::try_from(nal.len())
                .map_err(|_| "HEVC NAL length overflow")?
                .to_be_bytes(),
        );
        sample.extend_from_slice(nal);
        if let Some((offset, prefix)) = next {
            at = offset + prefix;
        } else {
            break;
        }
    }
    if !picture {
        return Err("HEVC encoder packet has no VCL picture".into());
    }
    let configuration = if sets.iter().all(|s| s.is_empty()) {
        None
    } else {
        if sets.iter().any(|s| s.len() != 1) {
            return Err("HEVC encoder configuration needs one complete VPS/SPS/PPS pair".into());
        }
        let vps = Vps::parse(sets[0][0], max_bytes).map_err(|e| e.to_string())?;
        let sps = Sps::parse(sets[1][0], max_bytes).map_err(|e| e.to_string())?;
        Pps::parse(sets[2][0], &sps, max_bytes).map_err(|e| e.to_string())?;
        if sps.vps_id != vps.id {
            return Err("HEVC SPS references a different VPS".into());
        }
        let profile = sps
            .profile
            .profile
            .ok_or("HEVC SPS is missing general profile")?;
        if !(8..=15).contains(&sps.depth[0])
            || !(8..=15).contains(&sps.depth[1])
            || sps.ordering.is_empty()
            || sps.ordering.len() > 7
        {
            return Err("HEVC configuration depth/temporal layers exceed hvcC fields".into());
        }
        let size = sets
            .iter()
            .try_fold(23usize, |n, entries| {
                n.checked_add(5)?.checked_add(entries[0].len())
            })
            .filter(|n| *n <= max_bytes)
            .ok_or("HEVC configuration exceeds byte limit")?;
        let mut config = Vec::new();
        config.try_reserve_exact(size).map_err(|e| e.to_string())?;
        config.extend_from_slice(&[
            1,
            (profile.space << 6) | (u8::from(profile.tier) << 5) | profile.idc,
        ]);
        config.extend_from_slice(&profile.compatibility.to_be_bytes());
        config.extend_from_slice(&profile.constraints.to_be_bytes()[2..]);
        config.extend_from_slice(&[
            sps.profile.level,
            0xf0,
            0,
            0xfc,
            0xfc | sps.chroma_format,
            0xf8 | (sps.depth[0] - 8),
            0xf8 | (sps.depth[1] - 8),
            0,
            0,
            ((sps.ordering.len() as u8) << 3) | (u8::from(sps.temporal_id_nesting) << 2) | 3,
            3,
        ]);
        for (index, entries) in sets.iter().enumerate() {
            let nal = entries[0];
            config.push(0x80 | 32 + index as u8);
            config.extend_from_slice(&1u16.to_be_bytes());
            config.extend_from_slice(
                &u16::try_from(nal.len())
                    .map_err(|_| "HEVC parameter NAL exceeds hvcC length")?
                    .to_be_bytes(),
            );
            config.extend_from_slice(nal);
        }
        HevcConfig::parse(&config).map_err(|e| e.to_string())?;
        Some(config)
    };
    Ok(Packet {
        configuration,
        sample,
        sync,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned_mp4::{Limits, Mp4Reader};
    use fvid_codecs::codec::{config::NalUnits, hevc_decoder::HevcDecoder};
    use std::io::Cursor;

    #[test]
    fn main_and_main10_annex_b_preserve_all_synthetic_ipb_pixels() {
        for (bytes, depth) in [
            (
                include_bytes!("../../../tests/fixtures/hevc/main-ipb.mp4").as_slice(),
                8,
            ),
            (
                include_bytes!("../../../tests/fixtures/hevc/main10-ipb.mp4").as_slice(),
                10,
            ),
        ] {
            let mut reader = Mp4Reader::open(Cursor::new(bytes), Limits::default()).unwrap();
            let original_configuration = reader.tracks()[0].configuration.clone();
            let config = HevcConfig::parse(&original_configuration).unwrap();
            let count = reader.tracks()[0].samples.len();
            let mut expected =
                HevcDecoder::from_configuration(&original_configuration, 64 << 20).unwrap();
            let mut actual = None;
            let mut decoded = 0;
            let mut packets = Vec::new();
            let mut saved_configuration = Vec::new();
            for index in 0..count {
                let mut original = Vec::new();
                reader.read_packet(0, index, &mut original).unwrap();
                let mut annex = vec![0];
                if index == 0 {
                    for array in &config.arrays {
                        if (32..=34).contains(&array.nal_type) {
                            for nal in &array.units {
                                annex.extend_from_slice(&[0, 0, 0, 1]);
                                annex.extend_from_slice(nal);
                            }
                        }
                    }
                }
                for nal in NalUnits::new(&original, config.length_size).unwrap() {
                    annex.extend_from_slice(&[0, 0, 1]);
                    annex.extend_from_slice(nal.unwrap());
                }
                let packet = convert(&annex, 1 << 20).unwrap();
                if index == 0 {
                    assert!(packet.sync);
                    let configuration = packet.configuration.as_ref().unwrap();
                    saved_configuration = configuration.clone();
                    let parsed = HevcConfig::parse(configuration).unwrap();
                    assert_eq!(parsed.bit_depth_luma, depth);
                    assert_eq!(parsed.length_size, 4);
                    actual =
                        Some(HevcDecoder::from_configuration(configuration, 64 << 20).unwrap());
                } else {
                    assert!(packet.configuration.is_none());
                }
                let a = actual
                    .as_mut()
                    .unwrap()
                    .decode_packet(&packet.sample)
                    .unwrap()
                    .unwrap();
                let b = expected.decode_packet(&original).unwrap().unwrap();
                assert_eq!((a.poc, a.output), (b.poc, b.output));
                assert_eq!(a.picture.dimensions, b.picture.dimensions);
                for (a, b) in a.picture.planes.iter().zip(&b.picture.planes) {
                    assert_eq!(a.samples(), b.samples());
                }
                packets.push(packet);
                decoded += 1;
            }
            assert_eq!(decoded, 17);
            let mut output = Cursor::new(Vec::new());
            let tracks = [crate::owned_matroska::TrackSpec {
                encoding: crate::owned_matroska::Encoding::Hevc {
                    configuration: &saved_configuration,
                    width: 64,
                    height: 64,
                },
                name: "",
                language: "und",
            }];
            let mut writer =
                crate::owned_matroska::PacketWriter::new(&mut output, &tracks).unwrap();
            for (index, packet) in packets.iter().enumerate() {
                writer
                    .write_packet(
                        0,
                        index as u64 * 33_333_333,
                        33_333_333,
                        packet.sync,
                        &packet.sample,
                    )
                    .unwrap();
            }
            writer.finish().unwrap();
            let mut saved = crate::owned_webm::WebmReader::open(
                Cursor::new(output.into_inner()),
                Default::default(),
            )
            .unwrap();
            saved.scan_all().unwrap();
            assert_eq!(saved.tracks[0].codec_private, saved_configuration);
            assert_eq!(saved.packets.len(), 17);
            for (index, packet) in packets.iter().enumerate() {
                assert_eq!(saved.read_packet(index).unwrap(), packet.sample);
            }
        }
    }

    #[test]
    fn framing_rejects_empty_reserved_and_over_budget_packets() {
        assert!(convert(&[], 100).is_err());
        assert!(convert(&[0, 0, 1], 100).is_err());
        assert!(convert(&[0, 0, 1, 0x80, 1, 0x80], 100).is_err());
        assert!(convert(&[0, 0, 1, 22 << 1, 1, 0x80], 100).is_err());
        assert!(convert(&[0, 0, 1, 2, 1, 0x80], 6).is_err());
        assert!(
            convert(&[0, 0, 1, 2, 1, 0x80], 7)
                .unwrap()
                .configuration
                .is_none()
        );
        assert!(convert(&[0, 0, 1, 32 << 1, 1, 0x80, 0, 0, 1, 2, 1, 0x80], 100).is_err());
    }
}
