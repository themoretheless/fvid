//! PS syntax acceptance is separate from pending HE-AACv2 PCM acceptance.
use fvid::{
    codec::{aac_native::NativeAacDecoder, config},
    container::mp4::{Limits, Mp4Reader},
};
use fvid_media::owned_aac::{aac_ps_data, aac_sbr_dsp, aac_sbr_history, bits::BitReader};

fn hex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(include_bytes!(
        "fixtures/playback-errors/aac-ps-syntax.json"
    ))
    .unwrap()
}

#[test]
fn original_sbr_ps_payloads_accept_all_three_packets_with_crc_and_retained_headers() {
    let reference = manifest();
    let mut sbr = aac_sbr_history::Stream::default();
    let mut ps = aac_ps_data::State::default();
    for (index, payload) in reference["sbr_payloads"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let raw = hex(payload.as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let kind = bits.read(4).unwrap();
        assert_eq!(kind, if index == 1 { 14 } else { 13 });
        let frame = sbr
            .read(&mut bits, raw.len() * 8, kind == 14, 48000, 16, 1)
            .unwrap();
        let extended = frame.syntax.data.extended_data.as_ref().unwrap();
        let parsed = ps.read_sbr_extensions(extended, 32).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].header_present, index == 0);
        assert_eq!(parsed[0].header.iid.unwrap().bands(), 20);
        assert_eq!(parsed[0].header.icc.unwrap().bands(), 20);
        assert_eq!(parsed[0].borders.len(), [1, 2, 0][index]);
        assert!(parsed[0].phase[0].enabled);
        // This check intentionally remains a refusal until real stereo DSP is
        // connected. Valid SBR/PS syntax must reach that exact pending stage.
        let error = aac_sbr_dsp::Dsp::default()
            .process(
                &frame,
                &[&[0.0; 1024]],
                48000,
                16,
                aac_sbr_dsp::OutputRate::Double,
            )
            .unwrap_err();
        assert_eq!(
            error.0,
            "SBR extended audio/PS synthesis is not yet implemented"
        );
    }
}

#[test]
fn original_explicit_and_implicit_ps_videos_reproduce_the_specific_pending_synthesis() {
    let fixtures: [&[u8]; 2] = [
        include_bytes!("fixtures/playback-errors/he-aac-ps-explicit-synthetic.mp4"),
        include_bytes!("fixtures/playback-errors/he-aac-ps-implicit-synthetic.mp4"),
    ];
    let reference = manifest();
    let packets = include_bytes!("fixtures/playback-errors/he-aac-ps-packets.bin");
    for (index, fixture) in fixtures.into_iter().enumerate() {
        let mut reader = Mp4Reader::open(std::io::Cursor::new(fixture), Limits::default()).unwrap();
        assert!(reader.refused().is_empty());
        assert_eq!(reader.tracks().len(), 2);
        assert!(
            reader
                .tracks()
                .iter()
                .any(|t| t.handler == *b"vide" && t.codec == *b"avc1" && !t.samples.is_empty())
        );
        let audio_index = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let track = &reader.tracks()[audio_index];
        assert_eq!(
            (track.sample_rate, track.channels, track.samples.len()),
            (48000, if index == 0 { 2 } else { 1 }, 3)
        );
        let asc = config::aac_specific_config(&track.configuration).unwrap();
        let parsed = config::AudioSpecificConfig::parse(asc).unwrap();
        assert_eq!(parsed.core.channels, 1);
        assert_eq!(parsed.output_sample_rate(), 48000);
        assert_eq!(
            parsed.ps_present,
            if index == 0 { Some(true) } else { None }
        );
        let mut decoder = if index == 0 {
            assert_eq!(
                NativeAacDecoder::new(asc).err().unwrap().to_string(),
                "AAC parametric stereo synthesis is not yet implemented"
            );
            None
        } else {
            Some(NativeAacDecoder::new(asc).unwrap())
        };
        let mut data = Vec::new();
        for (packet_index, row) in reference["packet_frames"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            reader
                .read_packet(audio_index, packet_index, &mut data)
                .unwrap();
            let offset = row["offset"].as_u64().unwrap() as usize;
            let length = row["bytes"].as_u64().unwrap() as usize;
            assert_eq!(data, &packets[offset..offset + length]);
            if packet_index == 0 {
                if let Some(decoder) = &mut decoder {
                    assert_eq!(
                        decoder.decode(&data).unwrap_err().to_string(),
                        "SBR extended audio/PS synthesis is not yet implemented"
                    );
                }
            }
        }
    }
}
