use std::{io::Cursor, path::Path};
#[test]
fn main_prediction_sbr_signalling_and_adts_transport_agree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("aac-main-sbr.json")).unwrap()).unwrap();
    let mut expected = Vec::new();
    for (i, case) in manifest["cases"].as_array().unwrap().iter().enumerate() {
        let data = std::fs::read(root.join(case["video"]["file"].as_str().unwrap())).unwrap();
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut pcm).unwrap();
        assert_eq!(pcm.len(), 12288 * 4);
        assert!(pcm
            .chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 1e-6));
        if i == 0 {
            expected = pcm;
        } else {
            assert_eq!(pcm, expected);
        }
    }
    let gold = include_bytes!("fixtures/playback-errors/aac-main-sbr-reference.f64le");
    assert_eq!(gold.len(), expected.len() * 2);
    let mut maximum_error = 0.0f64;
    for (i, (actual, reference)) in expected
        .chunks_exact(4)
        .zip(gold.chunks_exact(8))
        .enumerate()
    {
        let actual = f32::from_le_bytes(actual.try_into().unwrap()) as f64;
        let reference = f64::from_le_bytes(reference.try_into().unwrap());
        maximum_error = maximum_error.max((actual - reference).abs());
        assert!(
            (actual - reference).abs() < 1e-9,
            "sample {i}: {actual} vs scalar {reference}"
        );
    }
    eprintln!("Main/SBR maximum scalar PCM error: {maximum_error:e}");
    let control =
        include_bytes!("fixtures/playback-errors/aac-main-sbr-no-prediction-control.f64le");
    assert_eq!(control.len(), gold.len());
    assert!(
        expected
            .chunks_exact(4)
            .zip(control.chunks_exact(8))
            .any(|(actual, control)| {
                let actual = f32::from_le_bytes(actual.try_into().unwrap()) as f64;
                let control = f64::from_le_bytes(control.try_into().unwrap());
                (actual - control).abs() > 1e-5
            }),
        "acceptance must exercise active Main prediction through SBR"
    );
    for file in manifest["files"].as_array().unwrap() {
        let data = std::fs::read(root.join(file.as_str().unwrap())).unwrap();
        let mut pcm = Vec::new();
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(&data),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (48000, 1, 12288)
        );
        assert_eq!(pcm, expected);
        let mut root_pcm = Vec::new();
        fvid::native_media::decode_aac_pcm_interval(
            &data,
            &mut root_pcm,
            &Default::default(),
            None,
        )
        .unwrap();
        assert_eq!(root_pcm, expected);
        let mut out = Cursor::new(Vec::new());
        fvid::container::mp4_write::write_adts_aac(&data, &mut out).unwrap();
        let mut copied = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(out.get_ref(), &mut copied).unwrap();
        assert_eq!(copied, expected);
        let mut out = Cursor::new(Vec::new());
        fvid::container::mp4_write::write_adts_aac_reader(
            fvid::container::adts::StreamReader::open(Cursor::new(&data)).unwrap(),
            &mut out,
        )
        .unwrap();
        let mut sequential = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(out.get_ref(), &mut sequential).unwrap();
        assert_eq!(sequential, expected);
        let mut out = Cursor::new(Vec::new());
        fvid::container::matroska_write::write_adts(
            fvid::container::adts::StreamReader::open(Cursor::new(&data)).unwrap(),
            &mut out,
            None,
            None,
        )
        .unwrap();
        let mut mkv = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(out.get_ref(), &mut mkv, None)
            .unwrap();
        assert_eq!(mkv, expected);
    }
}

#[cfg(feature = "player")]
#[test]
fn main_sbr_player_clock_rewind_and_seek() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut pcm = Vec::new();
        while let Some(p) = reader.next_packet().unwrap() {
            assert_eq!(p.duration, 2048);
            if let Some(frame) = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
            {
                if let Some(f) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    pcm.extend(f.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(f) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                pcm.extend(f.data);
            }
        }
        pcm
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("aac-main-sbr.json")).unwrap()).unwrap();
    for file in manifest["files"].as_array().unwrap() {
        let data = std::fs::read(root.join(file.as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_aac::AacAudioReader::open(Cursor::new(data), Default::default())
                .unwrap();
        assert_eq!(
            (reader.sample_rate(), reader.timescale(), reader.channels()),
            (48000, 48000, 1)
        );
        let expected = play(&mut reader);
        assert_eq!(expected.len(), 12288 * 4);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for target in [1100, 6500, 12288] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), expected[landed as usize * 4..]);
        }
    }
}
