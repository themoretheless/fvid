use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-main-ps-adts.json"
    ))
    .unwrap()
}
fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut output = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            output.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    output
}

#[test]
fn main_ps_adts_matches_companion_pcm_and_repeated_ranges() {
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["adts"].as_str().unwrap());
        let mut expected = vec![];
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut expected,
        )
        .unwrap();
        let mut actual = vec![];
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(&data),
            &mut actual,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!((stats.channels, stats.sample_rate), (2, 48000));
        assert_eq!(actual, expected, "{}", c["name"]);
        for (from, to) in [(200, 280), (180, 220), (200, 280)] {
            let mut range = vec![];
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut range,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                range,
                expected[from as usize * 48 * 8..to as usize * 48 * 8]
            );
        }
    }
}
#[test]
fn main_ps_adts_player_rewind_seek_and_eof_preserve_stereo() {
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["adts"].as_str().unwrap());
        let mut expected = vec![];
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut expected,
        )
        .unwrap();
        let mut reader =
            fvid::playback_aac::AacAudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!((reader.channels(), reader.sample_rate()), (2, 48000));
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for target in [1100, 5800, 9000, (expected.len() / 8) as i64] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), expected[landed as usize * 8..]);
        }
    }
}

#[test]
fn late_main_ps_adts_prefix_admission_preserves_core_clock_until_ps() {
    let m = manifest();
    let c = &m["cases"][1];
    let data = bytes(c["adts"].as_str().unwrap());
    let core = bytes("aac-main-ps-core.f32le");
    for limit in [1, 3, 4, 5, 12] {
        let mut output = vec![];
        let options = fvid_control::CopyOptions {
            max_packets: Some(limit),
            ..Default::default()
        };
        let stats =
            fvid_media::owned_aac::decode_adts_pcm(Cursor::new(&data), &mut output, None, &options)
                .unwrap();
        let (rate, channels, samples) = if limit <= 4 {
            (24000, 1, limit * 1024)
        } else {
            (48000, 2, limit * 2048)
        };
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (rate, channels, samples)
        );
        assert_eq!(output.len(), samples as usize * channels as usize * 4);
        if limit <= 4 {
            for (a, b) in output.chunks_exact(4).zip(core.chunks_exact(4)) {
                assert!(
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap()))
                    .abs()
                        < 1e-8
                );
            }
        }
    }
    let mut range = vec![];
    let stats = fvid_media::owned_aac::decode_adts_pcm(
        Cursor::new(&data),
        &mut range,
        Some((Duration::from_millis(10), Duration::from_millis(60))),
        &Default::default(),
    )
    .unwrap();
    assert_eq!((stats.sample_rate, stats.channels), (24000, 1));
    for (a, b) in range
        .chunks_exact(4)
        .zip(core[240 * 4..1440 * 4].chunks_exact(4))
    {
        assert!(
            (f32::from_le_bytes(a.try_into().unwrap()) - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                < 1e-8
        );
    }
    assert_eq!(range.len(), 1200 * 4);
    let mut root = vec![];
    let mut owned = vec![];
    fvid::native_media::decode_aac_pcm_interval(&data, &mut root, &Default::default(), None)
        .unwrap();
    fvid_media::owned_aac::decode_adts_pcm(
        Cursor::new(&data),
        &mut owned,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(root, owned);
}
