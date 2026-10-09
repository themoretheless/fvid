use std::path::Path;
fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-late-sbr-fixed.json"
    ))
    .unwrap()
}
fn artifact(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn video(case: &serde_json::Value) -> Vec<u8> {
    artifact(case["video"]["file"].as_str().unwrap())
}
fn pcm(case: &serde_json::Value) -> Vec<u8> {
    let mut pcm = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm).unwrap();
    pcm
}
#[test]
fn main_lc_late_sbr_fixed_clock_matches_independent_waveform() {
    let m = manifest();
    for pair in m["cases"].as_array().unwrap().chunks_exact(2) {
        let control = pcm(&pair[0]);
        let core = artifact(pair[0]["reference"].as_str().unwrap());
        assert_eq!(control.len(), 6144 * 4);
        assert_eq!(core.len(), control.len());
        assert!(
            control
                .chunks_exact(4)
                .any(|s| f32::from_le_bytes(s.try_into().unwrap()).abs() > 1e-6)
        );
        for (a, b) in control.chunks_exact(4).zip(core.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-9
            );
        }
        let actual = pcm(&pair[1]);
        assert_eq!(actual.len(), 6144 * 4);
        assert_eq!(&actual[..3072 * 4], &control[..3072 * 4]);
        let gold = artifact(pair[1]["reference"].as_str().unwrap());
        assert_eq!(gold.len(), actual.len() * 2);
        for (i, (a, b)) in actual.chunks_exact(4).zip(gold.chunks_exact(8)).enumerate() {
            let a = f32::from_le_bytes(a.try_into().unwrap()) as f64;
            let b = f64::from_le_bytes(b.try_into().unwrap());
            assert!(
                (a - b).abs() < 1e-9,
                "{} sample {i}: {a} vs {b}",
                pair[1]["name"]
            );
        }
    }
}
#[test]
fn main_lc_fixed_clock_discovery_checkpoint_reset_and_eof_are_replayable() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let m = manifest();
    let blob = include_bytes!("fixtures/playback-errors/aac-late-sbr-fixed-packets.bin");
    for case in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["late"] == true)
    {
        let asc: Vec<u8> = case["asc"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
            .collect();
        let mut decoder = NativeAacDecoder::new_with_output_rate(&asc, 24000).unwrap();
        let expected = pcm(case);
        for replay in 0..2 {
            if replay > 0 {
                decoder.reset();
            }
            let mut actual = Vec::new();
            for (i, row) in case["frames"].as_array().unwrap().iter().enumerate() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let end = at + row["bytes"].as_u64().unwrap() as usize;
                let saved = decoder.checkpoint();
                assert!(decoder.decode_timed(&[], i as i64 * 1024, 1024).is_err());
                let first = decoder
                    .decode_timed(&blob[at..end], i as i64 * 1024, 1024)
                    .unwrap();
                decoder.restore(&saved).unwrap();
                let again = decoder
                    .decode_timed(&blob[at..end], i as i64 * 1024, 1024)
                    .unwrap();
                assert_eq!(first, again);
                let frame = again.unwrap();
                assert_eq!(frame.pts, (actual.len() / 4) as i64);
                assert_eq!(frame.duration, 1024);
                assert_eq!(decoder.sample_rate(), 24000);
                actual.extend(frame.samples.iter().flat_map(|s| s.to_le_bytes()));
            }
            assert!(decoder.finish().unwrap().is_none());
            assert!(decoder.finish().unwrap().is_none());
            assert_eq!(actual, expected);
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn main_lc_fixed_clock_late_sbr_player_seek_and_rewind_keep_pcm() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut pcm = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            if let Some(frame) = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
            {
                if let Some(frame) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    pcm.extend(frame.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(frame) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                pcm.extend(frame.data);
            }
        }
        pcm
    }
    let m = manifest();
    for case in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["late"] == true)
    {
        let data = video(case);
        let expected = pcm(case);
        let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(&data),
            Default::default(),
        )
        .unwrap();
        assert_eq!(reader.sample_rate(), 24000);
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for target in [1100, 3072, 4300, 6144] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), expected[landed as usize * 4..]);
        }
    }
}
