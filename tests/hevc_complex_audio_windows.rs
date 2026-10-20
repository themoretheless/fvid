//! General windows retain decoded samples even beyond compressed trim metadata.
use std::{io::Cursor, path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn pcm(data: &[u8], codec: &str, index: usize, window: Option<(Duration, Duration)>) -> Vec<u8> {
    let mut out = Vec::new();
    let o = fvid_control::CopyOptions {
        streams: vec![index],
        ..Default::default()
    };
    match codec {
        "A_AAC" => {
            fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(data),
                &mut out,
                window,
                &o,
            )
            .unwrap();
        }
        "A_OPUS" => {
            fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
                Cursor::new(data),
                &mut out,
                window,
                &o,
            )
            .unwrap();
        }
        "A_PCM/FLOAT/IEEE" => {
            fvid_media::owned_matroska_pcm::decode_matroska_pcm(
                Cursor::new(data),
                &mut out,
                window,
                &o,
            )
            .unwrap();
        }
        _ => panic!("unexpected codec"),
    };
    out
}
#[test]
fn complex_hevc_companion_windows_are_sample_exact_and_keep_video_clock() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("hevc-complex-audio-windows.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 6);
    for (serial, c) in m["cases"].as_array().unwrap().iter().enumerate() {
        let path = root().join(c["file"].as_str().unwrap());
        let source = std::fs::read(&path).unwrap();
        let mut reader =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(&source), Default::default())
                .unwrap();
        reader.scan_all().unwrap();
        let (from, to) = (
            c["interval_us"][0].as_i64().unwrap(),
            c["interval_us"][1].as_i64().unwrap(),
        );
        let window = Some((
            Duration::from_micros(from as u64),
            Duration::from_micros(to as u64),
        ));
        let mut expected = Vec::new();
        let mut trimmed = 0u64;
        for (i, t) in reader
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == 2)
        {
            let chosen = pcm(&source, &t.codec, i, window);
            let total = pcm(&source, &t.codec, i, None);
            trimmed += (total.len() - chosen.len()) as u64 / (t.channels * 4);
            expected.push((t.codec.clone(), t.channels, t.sample_rate, chosen));
        }
        assert!(expected.iter().all(|e| !e.3.is_empty()));
        let output = std::env::temp_dir().join(format!(
            "fvid-complex-audio-{}-{serial}.mkv",
            std::process::id()
        ));
        let stats = fvid_media::transcode_lossless(
            &path,
            &output,
            fvid_media::LosslessTransform {
                horizontal_flip: true,
                interval: Some((from, to)),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap_or_else(|e| panic!("{}: {e}", c["file"]));
        assert_eq!(
            stats.trimmed_audio_sample_frames, trimmed,
            "removed sample count"
        );
        let data = std::fs::read(&output).unwrap();
        let mut out =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        out.scan_all().unwrap();
        let audio: Vec<_> = out
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.kind == 2)
            .collect();
        assert_eq!(audio.len(), expected.len());
        for ((index, t), (source_codec, channels, rate, expected)) in
            audio.into_iter().zip(expected)
        {
            let keep_aac = c["variant"] == "long" && source_codec == "A_AAC";
            assert_eq!(
                t.codec,
                if keep_aac {
                    "A_AAC"
                } else {
                    "A_PCM/FLOAT/IEEE"
                }
            );
            assert_eq!((t.channels, t.sample_rate), (channels, rate));
            if !keep_aac {
                assert_eq!(t.codec_delay_ns, 0);
            }
            assert_eq!(
                pcm(&data, &t.codec, index, None),
                expected,
                "{} PCM window",
                c["file"]
            );
        }
        let video = out.tracks.iter().find(|t| t.kind == 1).unwrap();
        let original = reader.tracks.iter().find(|t| t.kind == 1).unwrap();
        let mut expected: Vec<_> = reader
            .packets
            .iter()
            .filter(|p| p.track == original.number && (from * 1000..to * 1000).contains(&p.pts_ns))
            .map(|p| p.pts_ns - from * 1000)
            .collect();
        expected.sort();
        assert_eq!(
            out.packets
                .iter()
                .filter(|p| p.track == video.number)
                .map(|p| p.pts_ns)
                .collect::<Vec<_>>(),
            expected
        );
        let mut pts: Vec<_> = reader
            .packets
            .iter()
            .filter(|p| p.track == original.number)
            .map(|p| p.pts_ns)
            .collect();
        pts.sort();
        let selected: Vec<_> = pts
            .iter()
            .enumerate()
            .filter(|(_, pts)| (from * 1000..to * 1000).contains(*pts))
            .map(|(i, _)| i)
            .collect();
        let number = video.number;
        let (w, h) = (video.width as usize, video.height as usize);
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
            w,
            h,
            &video.codec_private,
            32 << 20,
        )
        .unwrap();
        let packets: Vec<_> = out
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == number)
            .map(|(i, _)| i)
            .collect();
        for (&packet, &frame) in packets.iter().zip(&selected) {
            let decoded = decoder.decode(&out.read_packet(packet).unwrap()).unwrap();
            let unit = if decoded.depth == 8 { 1 } else { 2 };
            let gold = std::fs::read(root().join(c["frames"][frame].as_str().unwrap())).unwrap();
            let mut expected = Vec::new();
            let mut at = 0;
            for (pw, ph) in [(w, h), (w / 2, h / 2), (w / 2, h / 2)] {
                let len = pw * ph * unit;
                for row in gold[at..at + len].chunks_exact(pw * unit) {
                    for sample in row.chunks_exact(unit).rev() {
                        expected.extend_from_slice(sample)
                    }
                }
                at += len;
            }
            assert_eq!(
                decoded.frame.data, expected,
                "video samples alongside complex audio"
            );
        }
        std::fs::remove_file(output).unwrap();
        let prefix = format!("fvid-video-companions-{}-", std::process::id());
        assert!(
            !std::fs::read_dir(std::env::temp_dir())
                .unwrap()
                .filter_map(Result::ok)
                .any(|e| e.file_name().to_string_lossy().starts_with(&prefix)),
            "private PCM stages must be cleaned"
        );
    }
}
