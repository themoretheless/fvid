use std::io::Cursor;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(path(n)).unwrap()
}
fn path(n: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-open-ended-edit")
        .join(n)
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest-audio.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn compare(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    assert!(
        expected
            .chunks_exact(4)
            .any(|s| f32::from_le_bytes(s.try_into().unwrap()).abs() > 1e-5)
    );
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() <= 1e-6, "{a} != {b}");
    }
}
fn expected(c: &serde_json::Value, skip: usize) -> Vec<u8> {
    let stride = c["channels"].as_u64().unwrap() as usize * 4;
    let data = bytes(c["oracle"].as_str().unwrap());
    let start = c["start_frames"].as_u64().unwrap() as usize + skip;
    data[start * stride..].to_vec()
}
#[test]
fn audio_open_edit_probe_and_export_keep_nonzero_pcm_and_exact_lengths() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let p = fvid_media::owned_mp4_probe::probe_mp4(&path(n)).unwrap();
        assert_eq!(
            p.streams[1].duration,
            Some(c["media_ticks"].as_u64().unwrap() as i64),
            "{n}"
        );
        let mut out = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(n)),
            &mut out,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (
                c["rate"].as_u64().unwrap() as u32,
                c["channels"].as_u64().unwrap() as u16,
                c["frames"].as_u64().unwrap()
            ),
            "{n}"
        );
        compare(&out, &expected(&c, 0));
    }
}
#[test]
fn open_audio_packet_budget_errors_do_not_publish_pcm() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut out = Vec::new();
        let err = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(n)),
            &mut out,
            None,
            &fvid_media::CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("packet"), "{n}: {err}");
        assert!(out.is_empty(), "{n}");
    }
}
#[cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
#[cfg(feature = "player")]
#[test]
fn native_open_audio_preserves_pcm_seek_rewind_and_eof() {
    use fvid::audio::AudioStream;
    use std::time::Duration;
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let rate = c["rate"].as_u64().unwrap();
        assert_eq!(
            r.duration(),
            Some(Duration::from_nanos(
                c["frames"].as_u64().unwrap() * 1_000_000_000 / rate
            )),
            "{n}"
        );
        let pcm: Vec<_> = transitions::collect(&mut r)
            .into_iter()
            .flat_map(|f| f.3)
            .collect();
        compare(&pcm, &expected(&c, 0));
        assert!(transitions::collect(&mut r).is_empty());
        AudioStream::rewind(&mut r);
        let pcm: Vec<_> = transitions::collect(&mut r)
            .into_iter()
            .flat_map(|f| f.3)
            .collect();
        compare(&pcm, &expected(&c, 0));
        AudioStream::rewind(&mut r);
        let landed = r.seek_time(Duration::from_millis(10));
        // The existing finite single-AAC path returns the source media seek
        // coordinate; scheduled edits return a presentation coordinate. Check
        // both established conventions and the exact output timestamp below.
        let source_shift = if c["codec"] == "aac" && c["open"] == false {
            c["start_frames"].as_u64().unwrap() * u64::from(r.track().timescale) / rate
        } else {
            0
        };
        assert_eq!(
            landed,
            (u64::from(r.track().timescale) / 100 + source_shift) as i64,
            "{n}"
        );
        let frames = transitions::collect(&mut r);
        assert_eq!(frames.first().unwrap().2, Duration::from_millis(10), "{n}");
        let pcm: Vec<_> = frames.into_iter().flat_map(|f| f.3).collect();
        compare(&pcm, &expected(&c, rate as usize / 100));
    }
}

#[test]
fn fractional_open_audio_export_and_probe_match_the_exact_finite_clock() {
    let doc: serde_json::Value =
        serde_json::from_slice(&bytes("manifest-fractional.json")).unwrap();
    for c in doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["open"] == true)
    {
        let n = c["file"].as_str().unwrap();
        let known = n.replace("open.mp4", "known.mp4");
        let p = fvid_media::owned_mp4_probe::probe_mp4(&path(n)).unwrap();
        assert_eq!(p.streams[1].duration, Some(2645));
        let mut expected = Vec::new();
        let a = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(&known)),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        let mut actual = Vec::new();
        let b = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(n)),
            &mut actual,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            (a.sample_rate, a.channels, a.sample_frames),
            (b.sample_rate, b.channels, b.sample_frames)
        );
        compare(&actual, &expected);
    }
}
#[cfg(feature = "player")]
#[test]
fn fractional_open_native_audio_keeps_sample_alignment_without_rounding_movie_ticks() {
    use fvid::audio::AudioStream;
    use std::time::Duration;
    let doc: serde_json::Value =
        serde_json::from_slice(&bytes("manifest-fractional.json")).unwrap();
    for c in doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["open"] == true)
    {
        let n = c["file"].as_str().unwrap();
        let mut r = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            r.duration(),
            Some(Duration::from_nanos(2645u64 * 1_000_000_000 / 44100)),
            "{n}"
        );
        let frames = transitions::collect(&mut r);
        assert_eq!(frames.first().unwrap().2, Duration::from_nanos(102_324));
        let pcm: Vec<_> = frames.into_iter().flat_map(|f| f.3).collect();
        compare(&pcm, &bytes(c["oracle"].as_str().unwrap())[4..]);
        AudioStream::rewind(&mut r);
        r.seek_time(Duration::from_millis(10));
        let frames = transitions::collect(&mut r);
        assert_eq!(frames.first().unwrap().2, Duration::from_nanos(10_102_324));
        let pcm: Vec<_> = frames.into_iter().flat_map(|f| f.3).collect();
        compare(&pcm, &bytes(c["oracle"].as_str().unwrap())[81 * 4..]);
    }
}

#[test]
fn open_aac_packet_remux_preserves_nonzero_pcm_and_trimmed_audible_length() {
    for c in cases().into_iter().filter(|c| c["codec"] == "aac") {
        let n = c["file"].as_str().unwrap();
        let data = bytes(n);
        let mut input =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        let mut output = Cursor::new(Vec::new());
        fvid::container::matroska_write::write_mp4_aac(&mut input, 1, &mut output, None, None)
            .unwrap();
        let mut pcm = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(output.get_ref(), &mut pcm, None)
            .unwrap();
        compare(&pcm, &expected(&c, 0));
    }
}

#[test]
fn public_open_audio_plans_and_wave_publication_keep_exact_sample_counts() {
    struct Temporary(std::path::PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = std::env::temp_dir().join(format!(
        "fvid-open-edit-public-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let temp = Temporary(directory);
    for c in cases().into_iter().filter(|c| {
        c["open"] == true
            && c["start_frames"].as_u64().unwrap() > 0
            && c["file"].as_str().unwrap().contains("clock25-")
    }) {
        let n = c["file"].as_str().unwrap();
        let source = path(n);
        let plan = fvid_media::plan_decode_audio(&source, &Default::default(), &Default::default())
            .unwrap();
        assert_eq!(plan.streams.len(), 1);
        assert_eq!(plan.streams[0].index, 1);
        let native = fvid::native_plan::decode_audio(&source, &Default::default()).unwrap();
        assert_eq!(native.streams.len(), 1);
        assert_eq!(native.streams[0].index, 1);
        let output = temp.0.join(format!("{n}.wav"));
        let stats = fvid_media::decode_audio(&source, &output, &Default::default()).unwrap();
        assert_eq!(stats.sample_frames, c["frames"].as_u64().unwrap());
        let wav = std::fs::read(&output).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let mut at = 12;
        let mut bits = 0;
        let mut data = None;
        while at + 8 <= wav.len() {
            let size = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
            let body = &wav[at + 8..at + 8 + size];
            if &wav[at..at + 4] == b"fmt " {
                bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
            }
            if &wav[at..at + 4] == b"data" {
                data = Some(body.to_vec());
            }
            at += 8 + size + size % 2;
        }
        let pcm = data.unwrap();
        let f32_pcm = match bits {
            32 => pcm,
            64 => pcm
                .chunks_exact(8)
                .flat_map(|b| (f64::from_le_bytes(b.try_into().unwrap()) as f32).to_le_bytes())
                .collect(),
            _ => panic!("unexpected precision {bits}"),
        };
        compare(&f32_pcm, &expected(&c, 0));
        let rejected = temp.0.join(format!("{n}-rejected.wav"));
        let err = fvid_media::decode_audio(
            &source,
            &rejected,
            &fvid_media::CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("packet"), "{err}");
        assert!(!rejected.exists());
    }
    assert_eq!(
        std::fs::read_dir(&temp.0).unwrap().count(),
        6,
        "successful outputs only; no failed-publication temp artifacts"
    );
}
