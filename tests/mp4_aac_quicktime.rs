use fvid::container::mp4::{Limits as RootLimits, Mp4Reader as RootMp4Reader};
use fvid_media::owned_mp4::{Limits as OwnedLimits, Mp4Reader as OwnedMp4Reader};
use std::{io::Cursor, path::PathBuf};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-aac-quicktime")
}

fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(fixture_dir().join(n)).unwrap()
}

fn cases(k: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[k]
        .as_array()
        .unwrap()
        .clone()
}

fn assert_pcm_matches_reference(actual: &[u8], expected: &[u8], label: &str) {
    assert_eq!(actual.len(), expected.len(), "{label}: byte length");
    for (idx, (a, b)) in actual
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected.as_chunks::<4>().0.iter())
        .enumerate()
    {
        let va = f32::from_le_bytes(*a);
        let vb = f32::from_le_bytes(*b);
        assert!(
            (va - vb).abs() <= 1e-7,
            "{label} sample {idx}: actual {va} vs expected {vb}"
        );
    }
}

#[test]
fn root_mp4_reader_parses_all_quicktime_aac_controls_and_rejects_malformed() {
    for c in cases("controls") {
        let name = c["file"].as_str().unwrap();
        let has_video = c["has_video"].as_bool().unwrap();
        let data = bytes(name);
        let mut root = RootMp4Reader::open(Cursor::new(&data), RootLimits::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected_tracks = if has_video { 2 } else { 1 };
        assert_eq!(root.tracks().len(), expected_tracks, "{name}");
        let audio_idx = if has_video { 1 } else { 0 };
        let track = &root.tracks()[audio_idx];
        assert_eq!((track.handler, track.codec), (*b"soun", *b"mp4a"), "{name}");
        assert_eq!((track.sample_rate, track.channels), (48_000, 1), "{name}");
        assert_eq!(track.samples.len(), 3, "{name}");
        for s in 0..3 {
            let mut pkt = Vec::new();
            root.read_packet(audio_idx, s, &mut pkt).unwrap();
            assert!(!pkt.is_empty(), "{name} sample {s}");
        }
    }
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap();
        let expected = c["expected_error"].as_str().unwrap();
        let err = match RootMp4Reader::open(Cursor::new(bytes(name)), RootLimits::default()) {
            Ok(_) => panic!("{name}: expected error `{expected}` in RootMp4Reader"),
            Err(e) => e,
        };
        assert!(matches!(err, fvid::Error::Invalid(ref msg) if msg.contains(expected)), "{name}: {err}");
    }
}

#[test]
fn owned_mp4_quicktime_aac_controls_and_refusals_match_root_reader_and_analytical_pcm() {
    let reference_pcm = bytes("expected.f32le");
    for c in cases("controls") {
        let name = c["file"].as_str().unwrap();
        let has_video = c["has_video"].as_bool().unwrap();
        let data = bytes(name);
        let mut root = RootMp4Reader::open(Cursor::new(&data), RootLimits::default())
            .unwrap_or_else(|e| panic!("root {name}: {e}"));
        let mut owned = OwnedMp4Reader::open(Cursor::new(&data), OwnedLimits::default())
            .unwrap_or_else(|e| panic!("owned {name}: {e}"));
        assert_eq!(
            owned.tracks().len(),
            root.tracks().len(),
            "{name}: track count mismatch between OwnedMp4Reader and RootMp4Reader"
        );
        for t in 0..root.tracks().len() {
            let rt = &root.tracks()[t];
            let ot = &owned.tracks()[t];
            assert_eq!(
                (
                    ot.id,
                    ot.handler,
                    ot.codec,
                    ot.timescale,
                    ot.duration,
                    ot.sample_rate,
                    ot.channels,
                    &ot.configuration,
                    ot.samples.len()
                ),
                (
                    rt.id,
                    rt.handler,
                    rt.codec,
                    rt.timescale,
                    rt.duration,
                    rt.sample_rate,
                    rt.channels,
                    &rt.configuration,
                    rt.samples.len()
                ),
                "{name} track {t}"
            );
            for s in 0..rt.samples.len() {
                let mut rp = Vec::new();
                let mut op = Vec::new();
                root.read_packet(t, s, &mut rp).unwrap();
                owned.read_packet(t, s, &mut op).unwrap();
                assert_eq!(op, rp, "{name} track {t} sample {s}");
            }
        }
        let info = fvid_media::owned_mp4_probe::probe_mp4(&fixture_dir().join(name))
            .unwrap_or_else(|e| panic!("{name}: probe_mp4 failed: {e}"));
        assert_eq!(info.streams.len(), if has_video { 2 } else { 1 }, "{name}");
        let audio_stream = info.streams.last().unwrap();
        assert_eq!(audio_stream.codec, "aac", "{name}");
        assert_eq!(audio_stream.sample_rate, 48_000, "{name}");
        assert_eq!(audio_stream.channels, 1, "{name}");

        let mut decoded_pcm = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut decoded_pcm,
            None,
            &Default::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: decode_mp4_audio_pcm failed: {e}"));
        assert_eq!(stats.sample_rate, 48_000, "{name}");
        assert_eq!(stats.channels, 1, "{name}");
        assert_eq!(stats.sample_frames, 3_072, "{name}");
        assert_eq!(stats.decoded_frames, 3, "{name}");
        assert_pcm_matches_reference(&decoded_pcm, &reference_pcm, name);
    }
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap();
        let expected = c["expected_error"].as_str().unwrap();
        let err = match OwnedMp4Reader::open(Cursor::new(bytes(name)), OwnedLimits::default()) {
            Ok(_) => panic!("{name}: expected error `{expected}` in OwnedMp4Reader"),
            Err(e) => e,
        };
        assert!(!err.is_unsupported(), "{name}: {err}");
        assert!(err.to_string().contains(expected), "{name}: {err}");
    }
}

#[cfg(feature = "player")]
#[test]
fn player_audio_stream_decodes_quicktime_aac_controls_and_seeks_cleanly() {
    use fvid::audio::AudioStream;
    let reference_pcm = bytes("expected.f32le");
    for c in cases("controls") {
        let name = c["file"].as_str().unwrap();
        let data = bytes(name);
        let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(&data),
            RootLimits::default(),
        )
        .unwrap();
        assert_eq!(stream.codec(), "mp4a", "{name}");
        assert_eq!(stream.sample_rate(), 48_000, "{name}");
        assert_eq!(stream.channels(), 1, "{name}");
        let mut decoder = stream.make_decoder().unwrap();
        for pass in 0..2 {
            if pass == 1 {
                AudioStream::rewind(&mut stream);
                decoder.reset();
            }
            let mut out = Vec::new();
            while let Some(pkt) = stream.next_packet().unwrap() {
                let frame = decoder
                    .decode_packet(&pkt.data, pkt.pts, pkt.duration as u64)
                    .unwrap()
                    .unwrap();
                let presented = stream
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                    .unwrap();
                out.extend_from_slice(&presented.data);
            }
            assert!(stream.next_packet().unwrap().is_none());
            assert!(stream.next_packet().unwrap().is_none());
            assert_pcm_matches_reference(&out, &reference_pcm, &format!("{name} pass {pass}"));
        }
        assert_eq!(stream.seek_to(1_024), 1_024, "{name}");
        decoder.reset();
        let mut seek_out = Vec::new();
        while let Some(pkt) = stream.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&pkt.data, pkt.pts, pkt.duration as u64)
                .unwrap()
                .unwrap();
            if let Some(presented) = stream
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                seek_out.extend_from_slice(&presented.data);
            }
        }
        assert_pcm_matches_reference(
            &seek_out,
            &reference_pcm[1_024 * 4..],
            &format!("{name} seek(1024)"),
        );
    }
}

#[test]
fn owned_mp4_audio_interval_and_budget_controls_work_on_quicktime_v1_aac() {
    let reference_pcm = bytes("expected.f32le");
    let qt_v1_wave = bytes("qt-v1-wave.mov");
    let mut interval_pcm = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(&qt_v1_wave),
        &mut interval_pcm,
        Some((
            std::time::Duration::from_nanos(21_333_333),
            std::time::Duration::from_nanos(64_000_000),
        )),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.sample_rate, 48_000);
    assert_eq!(stats.channels, 1);
    assert_eq!(stats.sample_frames, 2_048);
    assert_pcm_matches_reference(
        &interval_pcm,
        &reference_pcm[1_024 * 4..],
        "qt-v1-wave.mov interval [1024..3072)",
    );

    let budget_err = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(&qt_v1_wave),
        &mut Vec::new(),
        None,
        &fvid_control::CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        budget_err
            .to_string()
            .contains("controlled memory budget exceeded"),
        "{budget_err}"
    );
}
