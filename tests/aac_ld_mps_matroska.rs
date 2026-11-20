use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-matroska")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn accept(c: &Value) {
    let bytes = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
    let mp4 = std::fs::read(root().join(c["mp4"].as_str().unwrap())).unwrap();
    let mut expected = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut expected).unwrap();
    let mut actual = Vec::new();
    let stats = fvid::native_media::decode_matroska_aac_pcm_interval(&bytes, &mut actual, None)
        .unwrap_or_else(|e| panic!("{} frontend: {e}", c["name"]));
    assert_eq!(
        stats.channels as u64,
        c["output_channels"].as_u64().unwrap()
    );
    assert_eq!(actual.len(), expected.len(), "{} length", c["name"]);
    assert!(actual == expected, "{} frontend PCM", c["name"]);
    let mut owned = Vec::new();
    fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
        Cursor::new(&bytes),
        &mut owned,
        None,
        &Default::default(),
    )
    .unwrap_or_else(|e| panic!("{} owned: {e}", c["name"]));
    assert!(owned == expected, "{} owned PCM", c["name"]);
}
#[test]
fn spatial_matroska_delay_bursts_play_instead_of_exceeding_three_source_windows() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 314);
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["group"] == 1)
    {
        accept(c);
    }
}
#[test]
fn two_core_spatial_frames_keep_their_matroska_source_windows() {
    let m = manifest();
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["group"] == 2)
    {
        accept(c);
    }
}

fn play(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Vec<u8> {
    let mut data = Vec::new();
    while let Some(p) = reader.next_packet().unwrap() {
        let stamp = reader.decoder_stamp(&p);
        if let Some(mut f) = reader.decode_source_packet(decoder, &p, stamp).unwrap() {
            if let Some(limit) = reader
                .decoded_packet_sample_limit(f.source_pts, f.source_duration)
                .unwrap()
            {
                let bytes = limit * usize::from(reader.channels()) * 4;
                assert!(bytes <= f.packet.data.len());
                f.packet.data.truncate(bytes);
            }
            if let Some(pcm) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                data.extend(pcm.data);
            }
        }
    }
    while let Some(mut f) = decoder.finish_packet().unwrap() {
        if let Some(limit) = reader
            .decoded_packet_sample_limit(f.source_pts, f.source_duration)
            .unwrap()
        {
            let bytes = limit * usize::from(reader.channels()) * 4;
            assert!(bytes <= f.packet.data.len());
            f.packet.data.truncate(bytes);
        }
        if let Some(pcm) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
            data.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    data
}

#[test]
fn matroska_stream_navigation_preserves_every_source_sample() {
    use fvid::audio::AudioStream;
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let mp4 = std::fs::read(root().join(c["mp4"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full).unwrap();
        let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        let mut decoder = reader.make_decoder().unwrap();
        let width = usize::from(reader.channels()) * 4;
        for stage in ["initial", "rewind"] {
            let actual = play(&mut reader, decoder.as_mut());
            assert_eq!(actual.len(), full.len(), "{} {stage} length", c["name"]);
            assert!(actual == full, "{} {stage} PCM", c["name"]);
            reader.rewind();
            decoder.reset();
        }
        for target in [
            13,
            4 * c["n"].as_u64().unwrap() + 7,
            7,
            (full.len() / width) as u64,
        ] {
            let ticks = (u128::from(target) * u128::from(reader.timescale())
                / u128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            decoder.reset();
            let first = ((landed.max(0) as u128 * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * width)
                .min(full.len());
            let actual = play(&mut reader, decoder.as_mut());
            assert_eq!(
                actual.len(),
                full.len() - first,
                "{} seek{target} length",
                c["name"]
            );
            assert!(actual == full[first..], "{} seek{target} PCM", c["name"]);
        }
        for _ in 0..2 {
            let rate = c["rate"].as_u64().unwrap();
            let start = std::time::Duration::from_nanos((13u64 * 1_000_000_000).div_ceil(rate));
            let end = std::time::Duration::from_nanos(
                ((4 * c["n"].as_u64().unwrap() + 7) * 1_000_000_000).div_ceil(rate),
            );
            let first =
                (start.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize * width;
            let last = (end.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize * width;
            let mut range = Vec::new();
            fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(&bytes),
                &mut range,
                Some((start, end)),
                &Default::default(),
            )
            .unwrap();
            assert!(range == full[first..last], "{} range PCM", c["name"]);
        }
    }
}

#[test]
fn core_only_controls_and_corrupt_spatial_suffixes_keep_their_distinct_outcomes() {
    let m = manifest();
    for c in m["controls"].as_array().unwrap() {
        accept(c);
    }
    for c in m["errors"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["asc_errors"].as_array().unwrap())
    {
        let bytes = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let expected = c["error"].as_str().unwrap();
        let mut actual = Vec::new();
        let err = fvid::native_media::decode_matroska_aac_pcm_interval(&bytes, &mut actual, None)
            .unwrap_err();
        assert!(
            err.to_string().contains(expected),
            "{} frontend: {err}",
            c["name"]
        );
        let mut actual = Vec::new();
        let err = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&bytes),
            &mut actual,
            None,
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(expected),
            "{} owned: {err}",
            c["name"]
        );
    }
}

#[test]
fn controlled_packet_boundaries_drain_complete_spatial_windows_and_refuse_incomplete_ones() {
    let m = manifest();
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mode"] == "plain")
    {
        let bytes = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&bytes),
            &mut full,
            None,
            &Default::default(),
        )
        .unwrap();
        let mut prefix = Vec::new();
        let stats = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&bytes),
            &mut prefix,
            None,
            &fvid_control::CopyOptions {
                max_packets: Some(4),
                ..Default::default()
            },
        )
        .unwrap();
        let samples = 4 * c["n"].as_u64().unwrap();
        assert_eq!(stats.sample_frames, samples);
        assert!(
            prefix == full[..(samples * c["output_channels"].as_u64().unwrap() * 4) as usize],
            "{} controlled prefix",
            c["name"]
        );
        if c["group"] == 2 {
            let mut prefix = Vec::new();
            let error = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(&bytes),
                &mut prefix,
                None,
                &fvid_control::CopyOptions {
                    max_packets: Some(1),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("unfinished LD-MPS PCM/spatial association"),
                "{error}"
            );
            assert!(prefix.is_empty());
        }
    }
}
