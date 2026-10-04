//! AVC encoder Annex B to owned MP4/Matroska packet framing.
use fvid_codecs::codec::{
    avc::{Pps, Sps},
    config::AvcConfig,
};
pub struct Packet {
    /// Present when this access unit contains SPS and PPS.
    pub configuration: Option<Vec<u8>>,
    /// Four-byte NAL lengths. Parameter NALs remain in-band.
    pub sample: Vec<u8>,
    pub sync: bool,
}
fn start(data: &[u8], from: usize) -> Option<(usize, usize)> {
    for index in from..data.len().saturating_sub(2) {
        if data[index..].starts_with(&[0, 0, 0, 1]) {
            return Some((index, 4));
        }
        if data[index..].starts_with(&[0, 0, 1]) {
            return Some((index, 3));
        }
    }
    None
}
pub fn convert(data: &[u8], max_bytes: usize) -> Result<Packet, String> {
    if data.is_empty() || data.len() > max_bytes {
        return Err("AVC Annex B input exceeds byte limit or is empty".into());
    }
    let (first, prefix) = start(data, 0).ok_or("AVC Annex B start code is missing")?;
    if data[..first].iter().any(|b| *b != 0) {
        return Err("AVC Annex B contains nonzero leading data".into());
    }
    let mut position = first + prefix;
    let mut sample = Vec::new();
    let mut sps = Vec::new();
    let mut pps = Vec::new();
    let mut sync = false;
    loop {
        let next = start(data, position);
        let mut end = next.map_or(data.len(), |entry| entry.0);
        while end > position && data[end - 1] == 0 {
            end -= 1;
        }
        let nal = &data[position..end];
        if nal.is_empty() || nal[0] & 0x80 != 0 || !(1..=23).contains(&(nal[0] & 31)) {
            return Err("AVC Annex B contains an empty or invalid NAL".into());
        }
        let kind = nal[0] & 31;
        if kind == 7 && !sps.contains(&nal) {
            sps.push(nal);
        }
        if kind == 8 && !pps.contains(&nal) {
            pps.push(nal);
        }
        sync |= kind == 5;
        let size = sample
            .len()
            .checked_add(4)
            .and_then(|n| n.checked_add(nal.len()))
            .ok_or("AVC sample size overflow")?;
        if size > max_bytes {
            return Err("AVC converted sample exceeds byte limit".into());
        }
        sample
            .try_reserve(size - sample.len())
            .map_err(|e| e.to_string())?;
        sample.extend_from_slice(
            &u32::try_from(nal.len())
                .map_err(|_| "AVC NAL size overflow")?
                .to_be_bytes(),
        );
        sample.extend_from_slice(nal);
        match next {
            Some((offset, prefix)) => position = offset + prefix,
            None => break,
        }
    }
    let configuration = if sps.is_empty() && pps.is_empty() {
        None
    } else {
        if sps.is_empty() || sps.len() > 31 || pps.is_empty() || pps.len() > 255 || sps[0].len() < 4
        {
            return Err("AVC configuration requires bounded SPS and PPS sets".into());
        }
        let mut config = vec![
            1,
            sps[0][1],
            sps[0][2],
            sps[0][3],
            255,
            224 | sps.len() as u8,
        ];
        for nal in &sps {
            config.extend_from_slice(
                &u16::try_from(nal.len())
                    .map_err(|_| "AVC SPS is too large")?
                    .to_be_bytes(),
            );
            config.extend_from_slice(nal);
        }
        config.push(pps.len() as u8);
        for nal in &pps {
            config.extend_from_slice(
                &u16::try_from(nal.len())
                    .map_err(|_| "AVC PPS is too large")?
                    .to_be_bytes(),
            );
            config.extend_from_slice(nal);
        }
        let parsed = AvcConfig::parse(&config).map_err(|e| e.to_string())?;
        let sets: Vec<_> = parsed
            .sps
            .iter()
            .map(|nal| Sps::parse(nal))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        if sets
            .iter()
            .enumerate()
            .any(|(index, set)| sets[..index].iter().any(|old| old.id == set.id))
        {
            return Err("AVC configuration has conflicting SPS identifiers".into());
        }
        let mut pps_ids = Vec::new();
        for nal in parsed.pps {
            let parameter = sets
                .iter()
                .find_map(|set| Pps::parse(nal, set).ok())
                .ok_or("AVC PPS has no valid sequence parameter set")?;
            if pps_ids.contains(&parameter.id) {
                return Err("AVC configuration has conflicting PPS identifiers".into());
            }
            pps_ids.push(parameter.id);
        }
        if config.len() > max_bytes {
            return Err("AVC configuration exceeds byte limit".into());
        }
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
    #[test]
    fn synthetic_encoder_framing_round_trips_owned_container_and_decoder() {
        use crate::{
            owned_matroska::{Encoding, PacketWriter, TrackSpec},
            owned_mp4::{Limits, Mp4Reader},
            owned_webm::WebmReader,
        };
        use fvid_codecs::codec::{avc_decoder::AvcDecoder, config::NalUnits};
        use std::io::Cursor;
        let mut reader = Mp4Reader::open(
            Cursor::new(include_bytes!(
                "../../../tests/fixtures/playback-errors/control.mp4"
            )),
            Limits::default(),
        )
        .unwrap();
        let original_config = reader.tracks()[0].configuration.clone();
        let config = AvcConfig::parse(&original_config).unwrap();
        let mut original = Vec::new();
        reader.read_packet(0, 0, &mut original).unwrap();
        let mut annex = vec![0]; // permitted leading zero
        for nal in config.sps.iter().chain(config.pps.iter()).copied().chain(
            NalUnits::new(&original, config.length_size)
                .unwrap()
                .map(Result::unwrap),
        ) {
            annex.extend_from_slice(&[0, 0, 0, 1]);
            annex.extend_from_slice(nal);
        }
        let packet = convert(&annex, 1 << 20).unwrap();
        assert!(packet.sync);
        let configuration = packet.configuration.as_ref().unwrap();
        let (width, height) = Sps::parse(config.sps[0]).unwrap().coded_dimensions();
        let mut output = Cursor::new(Vec::new());
        let tracks = [TrackSpec {
            encoding: Encoding::Avc {
                configuration,
                width,
                height,
            },
            name: "",
            language: "und",
        }];
        let mut writer = PacketWriter::new(&mut output, &tracks).unwrap();
        writer
            .write_packet(0, 0, 16_666_667, packet.sync, &packet.sample)
            .unwrap();
        writer.finish().unwrap();
        let mut saved =
            WebmReader::open(Cursor::new(output.into_inner()), Default::default()).unwrap();
        let payload = saved.read_packet(0).unwrap();
        assert_eq!(payload, packet.sample);
        let mut expected = AvcDecoder::new(&original_config, 16 << 20).unwrap();
        let mut actual = AvcDecoder::new(configuration, 16 << 20).unwrap();
        let mut expected_pixels = Vec::new();
        let mut actual_pixels = Vec::new();
        expected
            .decode_order(&original)
            .unwrap()
            .unwrap()
            .write_planar(&mut expected_pixels)
            .unwrap();
        actual
            .decode_order(&payload)
            .unwrap()
            .unwrap()
            .write_planar(&mut actual_pixels)
            .unwrap();
        assert_eq!(actual_pixels, expected_pixels);
    }
    #[test]
    fn framing_is_bounded_and_rejects_invalid_start_codes_and_empty_nals() {
        assert!(convert(&[0, 0, 1, 0x41, 0x80], 5).is_err()); // converted length needs six bytes
        assert!(
            convert(&[0, 0, 1, 0x41, 0x80], 6)
                .unwrap()
                .configuration
                .is_none()
        );
        assert!(convert(&[1, 0, 0, 1, 0x41, 0x80], 100).is_err());
        assert!(convert(&[0, 0, 1, 0, 0, 1], 100).is_err());
        assert!(convert(&[0, 0, 1, 0x81], 100).is_err());
    }
}
