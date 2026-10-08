//! PS syntax acceptance is separate from pending HE-AACv2 PCM acceptance.
use fvid::{
    codec::{aac_native::NativeAacDecoder, config},
    container::mp4::{Limits, Mp4Reader},
};
use fvid_media::owned_aac::{
    aac_ps_data, aac_ps_history, aac_sbr_dsp, aac_sbr_history, bits::BitReader,
};

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
    let mut ps = aac_ps_history::Stream::default();
    let mut retained = None;
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
        assert_eq!(parsed[0].syntax.header_present, index == 0);
        assert_eq!(parsed[0].syntax.header.iid.unwrap().bands(), 20);
        assert_eq!(parsed[0].syntax.header.icc.unwrap().bands(), 20);
        assert_eq!(parsed[0].syntax.borders.len(), [1, 2, 0][index]);
        assert!(parsed[0].syntax.phase[0].enabled);
        assert!(parsed[0].parameters.initialized);
        let values = parsed[0].parameters.retained.dequantize().unwrap();
        assert_eq!(values.iid_db, vec![4.0; 20]);
        assert_eq!(values.coherence, vec![0.84118; 20]);
        assert_eq!(values.ipd_radians, vec![std::f64::consts::FRAC_PI_4; 11]);
        assert_eq!(values.opd_radians, values.ipd_radians);
        if index == 2 {
            assert_eq!(retained.as_ref(), Some(&parsed[0].parameters.retained));
        }
        retained = Some(parsed[0].parameters.retained.clone());
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

fn history_manifest() -> serde_json::Value {
    serde_json::from_slice(include_bytes!(
        "fixtures/playback-errors/aac-ps-history-oracles.json"
    ))
    .unwrap()
}

#[test]
fn original_varying_ps_video_reconstructs_native_parameters_before_pending_stereo_synthesis() {
    let reference = history_manifest();
    let fixture = include_bytes!("fixtures/playback-errors/he-aac-ps-varying-synthetic.mp4");
    let packets = include_bytes!("fixtures/playback-errors/he-aac-ps-varying-packets.bin");
    let mut reader =
        Mp4Reader::open(std::io::Cursor::new(fixture.as_slice()), Limits::default()).unwrap();
    assert!(reader.refused().is_empty());
    assert_eq!(reader.tracks().len(), 2);
    let track = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    assert_eq!(
        (
            reader.tracks()[track].sample_rate,
            reader.tracks()[track].channels,
            reader.tracks()[track].samples.len()
        ),
        (48000, 2, 3)
    );
    let mut sbr = aac_sbr_history::Stream::default();
    let mut ps = aac_ps_history::Stream::default();
    let mut data = Vec::new();
    for index in 0..3 {
        reader.read_packet(track, index, &mut data).unwrap();
        let row = &reference["packet_frames"][index];
        let offset = row["offset"].as_u64().unwrap() as usize;
        let bytes = row["bytes"].as_u64().unwrap() as usize;
        assert_eq!(data, &packets[offset..offset + bytes]);
        let raw = hex(reference["sbr_payloads"][index].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let kind = bits.read(4).unwrap();
        let frame = sbr
            .read(&mut bits, raw.len() * 8, kind == 14, 48000, 16, 1)
            .unwrap();
        let parsed = ps
            .read_sbr_extensions(frame.syntax.data.extended_data.as_ref().unwrap(), 32)
            .unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].parameters.initialized);
        assert_eq!(parsed[0].parameters.envelopes.len(), 2);
        for (values, target) in parsed[0]
            .parameters
            .envelopes
            .iter()
            .zip(reference["video_targets"][index].as_array().unwrap())
        {
            for (actual, name) in [
                (&values.iid, "iid"),
                (&values.icc, "icc"),
                (&values.ipd, "ipd"),
                (&values.opd, "opd"),
            ] {
                assert_eq!(
                    actual,
                    &target[name]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_i64().unwrap() as i16)
                        .collect::<Vec<_>>()
                );
            }
            let levels = values.dequantize().unwrap();
            assert!(levels.iid_db.iter().any(|&v| v < 0.0));
            assert!(levels.iid_db.iter().any(|&v| v > 0.0));
            assert!(levels.coherence.iter().any(|&v| v < 0.0));
        }
        assert_eq!(
            aac_sbr_dsp::Dsp::default()
                .process(
                    &frame,
                    &[&[0.0; 1024]],
                    48000,
                    16,
                    aac_sbr_dsp::OutputRate::Double
                )
                .unwrap_err()
                .0,
            "SBR extended audio/PS synthesis is not yet implemented"
        );
    }
}

#[test]
fn original_malformed_ps_index_videos_reach_the_intended_numeric_refusal() {
    let fixtures: [&[u8]; 3] = [
        include_bytes!("fixtures/playback-errors/he-aac-ps-invalid-iid-coarse-synthetic.mp4"),
        include_bytes!("fixtures/playback-errors/he-aac-ps-invalid-iid-fine-synthetic.mp4"),
        include_bytes!("fixtures/playback-errors/he-aac-ps-invalid-icc-synthetic.mp4"),
    ];
    let reference = history_manifest();
    let packets = include_bytes!("fixtures/playback-errors/he-aac-ps-invalid-packets.bin");
    for (fixture, case) in fixtures
        .into_iter()
        .zip(reference["malformed"].as_array().unwrap())
    {
        let mut reader = Mp4Reader::open(std::io::Cursor::new(fixture), Limits::default()).unwrap();
        assert!(reader.refused().is_empty());
        assert_eq!(reader.tracks().len(), 2);
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        assert_eq!(
            (
                reader.tracks()[track].sample_rate,
                reader.tracks()[track].channels,
                reader.tracks()[track].samples.len()
            ),
            (48000, 2, 3)
        );
        let mut sbr = aac_sbr_history::Stream::default();
        let mut ps = aac_ps_history::Stream::default();
        let mut syntax = aac_ps_data::State::default();
        let mut data = Vec::new();
        for index in 0..3 {
            reader.read_packet(track, index, &mut data).unwrap();
            let row = &case["packet_frames"][index];
            let offset = row["offset"].as_u64().unwrap() as usize;
            let bytes = row["bytes"].as_u64().unwrap() as usize;
            assert_eq!(data, &packets[offset..offset + bytes]);
            let raw = hex(case["sbr_payloads"][index].as_str().unwrap());
            let mut bits = BitReader::new(&raw);
            let kind = bits.read(4).unwrap();
            let frame = sbr
                .read(&mut bits, raw.len() * 8, kind == 14, 48000, 16, 1)
                .unwrap();
            let area = frame.syntax.data.extended_data.as_ref().unwrap();
            assert_eq!(syntax.read_sbr_extensions(area, 32).unwrap().len(), 1);
            let before = ps.clone();
            assert_eq!(
                ps.read_sbr_extensions(area, 32).unwrap_err().0,
                case["error"].as_str().unwrap()
            );
            assert_eq!(ps, before);
        }
    }
}

#[test]
fn original_no_envelope_mode_change_video_preserves_current_modes_separately_from_old_native_grids()
{
    let reference = history_manifest();
    let reference = &reference["mode_transition"];
    let fixture =
        include_bytes!("fixtures/playback-errors/he-aac-ps-mode-transition-synthetic.mp4");
    let packets = include_bytes!("fixtures/playback-errors/he-aac-ps-mode-transition-packets.bin");
    let mut reader =
        Mp4Reader::open(std::io::Cursor::new(fixture.as_slice()), Limits::default()).unwrap();
    assert!(reader.refused().is_empty());
    assert_eq!(reader.tracks().len(), 2);
    let track = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    assert_eq!(
        (
            reader.tracks()[track].sample_rate,
            reader.tracks()[track].channels,
            reader.tracks()[track].samples.len()
        ),
        (48000, 2, 3)
    );
    let mut sbr = aac_sbr_history::Stream::default();
    let mut ps = aac_ps_history::Stream::default();
    let mut packet = Vec::new();
    for index in 0..3 {
        reader.read_packet(track, index, &mut packet).unwrap();
        let row = &reference["packet_frames"][index];
        let offset = row["offset"].as_u64().unwrap() as usize;
        let bytes = row["bytes"].as_u64().unwrap() as usize;
        assert_eq!(packet, &packets[offset..offset + bytes]);
        let raw = hex(reference["sbr_payloads"][index].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let kind = bits.read(4).unwrap();
        let sbr_frame = sbr
            .read(&mut bits, raw.len() * 8, kind == 14, 48000, 16, 1)
            .unwrap();
        let frames = ps
            .read_sbr_extensions(sbr_frame.syntax.data.extended_data.as_ref().unwrap(), 32)
            .unwrap();
        assert_eq!(frames.len(), 1);
        let values = &frames[0].parameters;
        let expected = &reference["expected"][index];
        assert!(values.initialized);
        assert_eq!(
            u64::from(values.iid_mode.value()),
            expected["iid_mode"].as_u64().unwrap()
        );
        assert_eq!(
            u64::from(values.icc_mode.value()),
            expected["icc_mode"].as_u64().unwrap()
        );
        assert_eq!(values.envelopes.len(), if index == 0 { 1 } else { 0 });
        assert_eq!(
            (
                values.retained.iid_mode.bands(),
                values.retained.icc_mode.bands()
            ),
            (10, 10)
        );
        for (actual, name) in [
            (&values.retained.iid, "iid"),
            (&values.retained.icc, "icc"),
            (&values.retained.ipd, "ipd"),
            (&values.retained.opd, "opd"),
        ] {
            assert_eq!(
                actual,
                &expected["retained"][name]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_i64().unwrap() as i16)
                    .collect::<Vec<_>>()
            );
        }
        if index > 0 {
            assert_eq!((values.iid_mode.bands(), values.icc_mode.bands()), (34, 34));
        }
        let physical = values.retained.dequantize().unwrap();
        if index == 2 {
            assert!(
                values.header.iid.is_none() && values.header.icc.is_none() && !values.phase_enabled
            );
            assert_eq!(physical.iid_db, vec![0.0; 10]);
            assert_eq!(physical.coherence, vec![1.0; 10]);
            assert_eq!(physical.ipd_radians, vec![0.0; 5]);
            assert_eq!(physical.opd_radians, vec![0.0; 5]);
        }
    }
}
