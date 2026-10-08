//! Original PS export repeats exercise retained lookahead, source EOF and checkpoints.
use fvid_control::CopyOptions;
use std::{io::Cursor, time::Duration};
#[test]
fn core_clock_ps_mp4_export_retains_source_ticks_and_negotiates_output_wav_geometry() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-core-clock-oracles.json"
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
        let reader =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&file), Default::default()).unwrap();
        let track = reader
            .tracks()
            .iter()
            .find(|t| t.handler == *b"soun")
            .unwrap();
        assert_eq!((track.timescale, track.sample_rate), (24000, 48000));
        assert_eq!(
            track.samples.get(0).unwrap().duration,
            if c["reference"] == 13 { 960 } else { 1024 }
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
        let info = fvid_media::owned_wave_inspect::inspect(
            &mut std::fs::File::open(&destination).unwrap(),
            None,
        )
        .unwrap();
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
    }
}
