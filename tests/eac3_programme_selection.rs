#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod support;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{presentation::PresentationDecoder, CompressionMode, Eac3Decoder},
};
use std::{io::Cursor, path::Path, time::Duration};
use support::DecodeComplete;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-programme-selection")
            .join(name),
    )
    .unwrap()
}
fn open(name: &str) -> fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>> {
    fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(read(name)), Default::default())
        .unwrap()
}
fn collect(
    stream: &mut dyn AudioStream,
    programme: u8,
    mode: CompressionMode,
    normalize: bool,
) -> Vec<(Duration, Vec<u8>)> {
    let mut d = Eac3Decoder::for_container_with_compression(
        stream.extra_data(),
        stream.sample_rate(),
        stream.channels(),
        mode,
    )
    .unwrap()
    .with_dialogue_normalization(normalize)
    .with_programme(programme)
    .unwrap();
    let mut out = Vec::new();
    while let Some(step) = stream.next_step().unwrap() {
        if let AudioStep::Encoded(p) = step {
            if let Some(p) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(p) = stream.present_decoded(p.packet, p.source_pts).unwrap() {
                    out.push((p.presentation_time(), p.data));
                }
            }
        }
    }
    out.extend(
        d.drain_presented(stream)
            .into_iter()
            .map(|p| (p.presentation_time(), p.data)),
    );
    stream.validate_eof().unwrap();
    out
}
#[test]
fn all_independent_programmes_select_own_pcm_history_without_extra_time() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["accepted"], true);
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 78);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize;
        let rate = c["rate"].as_u64().unwrap() as u32;
        let raw = read(&format!("{name}.ec3"));
        for mode in [CompressionMode::Line, CompressionMode::Rf] {
            let mut previous = None;
            for programme in 0..8u8 {
                let control = c["controls"][programme as usize].as_str().unwrap();
                let expected = collect(&mut open(&format!("{control}.mkv")), 0, mode, true);
                assert_eq!(expected.len(), 3);
                assert!(expected
                    .iter()
                    .flat_map(|(_, b)| b.chunks_exact(4))
                    .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 1e-7));
                if let Some(previous) = previous {
                    assert_ne!(
                        expected, previous,
                        "{name}: controls must distinguish choices"
                    );
                }
                previous = Some(expected.clone());
                let bytes: Vec<u8> = expected.iter().flat_map(|(_, b)| b.clone()).collect();
                for (i, (pts, b)) in expected.iter().enumerate() {
                    assert_eq!(
                        *pts,
                        Duration::from_nanos(
                            i as u64 * c["blocks"].as_u64().unwrap() * 256 * 1_000_000_000
                                / u64::from(rate)
                        )
                    );
                    assert_eq!(
                        b.len(),
                        c["blocks"].as_u64().unwrap() as usize * 256 * width * 4
                    );
                }
                let mut d = Eac3Decoder::new(&[], rate, width as u16)
                    .unwrap()
                    .with_compression(mode)
                    .with_dialogue_normalization(true)
                    .with_programme(programme)
                    .unwrap();
                for attempt in 0..2 {
                    let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
                    assert!(
                        got == bytes[256 * width * 4..],
                        "{name}: programme{programme} raw PCM differs"
                    );
                    if attempt == 0 {
                        d.reset();
                    }
                }
                let probe =
                    fvid::codec::eac3_decoder::checked_programme_frame(&raw, programme).unwrap();
                assert_eq!(probe.sample_rate, rate);
                let source = read(&format!("{control}.ec3"));
                assert_eq!(
                    fvid::codec::eac3_decoder::speaker_mask_for_programme(
                        &raw,
                        width as u16,
                        programme
                    )
                    .unwrap(),
                    fvid::codec::eac3_decoder::speaker_mask(&source, width as u16).unwrap()
                );
                if programme == 7 {
                    let mut switched = d.with_programme(0).unwrap();
                    let control0 = c["controls"][0].as_str().unwrap();
                    let expected0 = collect(&mut open(&format!("{control0}.mkv")), 0, mode, true);
                    let bytes0: Vec<u8> = expected0.into_iter().flat_map(|(_, b)| b).collect();
                    assert_eq!(
                        switched.decode_complete(&raw, 0, 0).unwrap().unwrap().data,
                        bytes0[256 * width * 4..],
                        "{name}: switching resets finished/history while retaining policy"
                    );
                }
                let mut stream = open(&format!("{name}.mkv"));
                assert_eq!(
                    collect(&mut stream, programme, mode, true),
                    expected,
                    "{name}: programme{programme} Matroska"
                );
                stream.rewind();
                assert_eq!(collect(&mut stream, programme, mode, true), expected);
                if c["expanded"] == true {
                    let mut direct = PresentationDecoder::for_container(rate, width as u16)
                        .unwrap()
                        .with_programme(programme)
                        .unwrap()
                        .with_dialogue_normalization(true);
                    direct.set_dolby_compression(mode);
                    assert_eq!(
                        direct.decode_complete(&raw, 0, 0).unwrap().unwrap().data,
                        bytes
                    );
                }
            }
        }
    }
}
#[test]
fn out_of_range_programme_is_rejected_before_decode() {
    for id in [8, 255] {
        assert!(Eac3Decoder::new(&[], 48000, 2)
            .unwrap()
            .with_programme(id)
            .is_err());
        assert!(PresentationDecoder::new(48000, 8)
            .unwrap()
            .with_programme(id)
            .is_err());
    }
}

#[test]
fn unselected_payloads_are_opaque_and_do_not_consume_selected_history() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let original = read(&format!("{name}.ec3"));
        let width = c["channels"].as_u64().unwrap() as usize;
        let rate = c["rate"].as_u64().unwrap() as u32;
        for programme in [1, 7] {
            let mut corrupt = original.clone();
            let mut offset = 0;
            let mut current = 0;
            while offset < corrupt.len() {
                let kind = corrupt[offset + 2] >> 6;
                let id = (corrupt[offset + 2] >> 3) & 7;
                let size =
                    ((corrupt[offset + 2] as usize & 7) * 256 + corrupt[offset + 3] as usize + 1)
                        * 2;
                if kind != 1 {
                    current = id;
                }
                if current != programme {
                    corrupt[offset + 6..offset + size].fill(0xa5);
                }
                offset += size;
            }
            let control = c["controls"][programme as usize].as_str().unwrap();
            let expected = collect(
                &mut open(&format!("{control}.mkv")),
                0,
                CompressionMode::Rf,
                true,
            );
            let bytes: Vec<u8> = expected.into_iter().flat_map(|(_, b)| b).collect();
            let mut d = Eac3Decoder::new(&[], rate, width as u16)
                .unwrap()
                .with_compression(CompressionMode::Rf)
                .with_dialogue_normalization(true)
                .with_programme(programme)
                .unwrap();
            let unselected = read(&format!("{}.ec3", c["controls"][0].as_str().unwrap()));
            assert!(
                d.decode_encoded(&unselected, 0, 0).unwrap().is_none(),
                "{name}: unselected-only input emits nothing"
            );
            assert_eq!(
                d.decode_complete(&corrupt, 0, 0).unwrap().unwrap().data,
                bytes[256 * width * 4..],
                "{name}: unselected payload must not touch PCM/RNG/history"
            );
            assert!(
                fvid::codec::eac3_decoder::checked_programme_frame(&corrupt, programme).is_some()
            );
        }
    }
}
