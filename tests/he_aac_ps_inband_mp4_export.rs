//! Original PS export repeats exercise retained lookahead, source EOF and checkpoints.
use fvid_control::CopyOptions;
use std::{io::Cursor, time::Duration};
#[test]
fn implicit_mp4_ps_edits_export_independent_pcm_and_negotiated_wav_geometry() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-inband-mp4-export-oracles.json"
    ))
    .unwrap();
    let references: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-dsp-oracles.json"
    ))
    .unwrap();
    let data = include_bytes!("fixtures/playback-errors/aac-ps-dsp-reference.bin");
    for c in manifest["cases"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["file"].as_str().unwrap());
        let file = std::fs::read(&path).unwrap();
        let mut legacy_reader =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&file), Default::default()).unwrap();
        let index = legacy_reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let track = legacy_reader.tracks()[index].clone();
        assert_eq!(track.channels, 1);
        let mut legacy = fvid_media::owned_aac::NativeAacDecoder::new_with_output_rate(
            &hex(c["asc"].as_str().unwrap()),
            48000,
        )
        .unwrap();
        let mut packet = vec![];
        legacy_reader.read_packet(index, 0, &mut packet).unwrap();
        assert!(
            legacy
                .decode(&packet)
                .unwrap_err()
                .to_string()
                .contains("SBR extended audio/PS synthesis is not yet implemented")
        );
        let mut source = vec![];
        for frame in references["cases"][c["reference"].as_u64().unwrap() as usize]["frames"]
            .as_array()
            .unwrap()
        {
            let n = frame["Double"][0][1].as_u64().unwrap() as usize;
            for i in 0..n {
                for channel in 0..2 {
                    let off = frame["Double"][channel][0].as_u64().unwrap() as usize + i * 8;
                    source.push(
                        (f64::from_le_bytes(data[off..off + 8].try_into().unwrap()) / 32768.)
                            as f32,
                    );
                }
            }
        }
        let mut expected = vec![0.; 1600 * 2];
        expected.extend_from_slice(&source[1920 * 2..5120 * 2]);
        expected.extend_from_slice(&source[1920 * 2..5120 * 2]);
        expected.extend_from_slice(&vec![0.; 1600 * 2]);
        for (interval, options, from, to) in [
            (None, CopyOptions::default(), 0, 9600),
            (
                Some((Duration::from_millis(50), Duration::from_millis(180))),
                CopyOptions::default(),
                2400,
                8640,
            ),
            (
                None,
                CopyOptions {
                    max_packets: Some(3),
                    ..Default::default()
                },
                0,
                4800,
            ),
            (
                None,
                CopyOptions {
                    max_controlled_bytes: Some(128 * 1024 * 1024),
                    ..Default::default()
                },
                0,
                9600,
            ),
        ] {
            let mut output = vec![];
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&file),
                &mut output,
                interval,
                &options,
            )
            .unwrap();
            assert_eq!(
                (stats.sample_rate, stats.channels, stats.sample_frames),
                (48000, 2, (to - from) as u64)
            );
            assert_eq!(output.len(), (to - from) * 8);
            for (bytes, e) in output.chunks_exact(4).zip(&expected[from * 2..to * 2]) {
                let a = f32::from_le_bytes(bytes.try_into().unwrap());
                assert!(
                    (a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16,
                    "{}: {a} != {e}",
                    c["file"]
                );
            }
            if options.max_packets.is_none() {
                let reader =
                    fvid::container::mp4::Mp4Reader::open(Cursor::new(&file), Default::default())
                        .unwrap();
                let mut root = vec![];
                let stats =
                    fvid::native_media::decode_mp4_aac_reader(reader, &mut root, interval).unwrap();
                assert_eq!(stats.sample_frames, (to - from) as u64);
                assert_eq!(root, output);
            }
        }
        let destination = std::env::temp_dir().join(format!(
            "fvid-mp4-ps-export-{}-{}.wav",
            std::process::id(),
            c["reference"]
        ));
        let _ = std::fs::remove_file(&destination);
        let stats = fvid_media::decode_audio(&path, &destination, &CopyOptions::default()).unwrap();
        assert_eq!(stats.sample_frames, 9600);
        let mut wave = std::fs::File::open(&destination).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut wave, None).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.sample_frames),
            (48000, 2, 9600)
        );
        let source_info = fvid::native_media::audio_source_info_selected(&path, None).unwrap();
        assert_eq!(
            (
                source_info.sample_rate,
                source_info.channels,
                source_info.channel_mask
            ),
            (48000, 2, Some(3))
        );
        std::fs::remove_file(destination).unwrap();
        for budget in [1, 10 * 1024 * 1024] {
            let mut output = vec![];
            let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&file),
                &mut output,
                None,
                &CopyOptions {
                    max_controlled_bytes: Some(budget),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(error.to_string().contains("controlled memory budget"));
            assert!(output.is_empty());
        }
    }
}

fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
