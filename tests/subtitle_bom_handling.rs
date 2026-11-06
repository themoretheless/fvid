#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};

fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/subtitle-bom-handling")
            .join(n),
    )
    .unwrap()
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}

#[test]
fn matroska_bom_prefixed_subtitle_blocks_strip_bom_and_preserve_video() {
    let m = manifest();
    for c in m["mkv_cases"].as_array().unwrap() {
        let n = c["file"].as_str().unwrap();
        let wanted_texts: Vec<String> = c["texts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let mut r = fvid::playback_webm_subtitles::WebmSubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        assert_eq!(cues.len(), 3, "{n}");
        for (i, cue) in cues.iter().enumerate() {
            assert_eq!(cue.text, wanted_texts[i], "{n} cue {i}");
            assert!(!cue.text.contains('\u{feff}'), "{n} cue {i} leaked BOM");
            assert_eq!(
                (cue.start.as_nanos(), cue.end.as_nanos()),
                (i as u128 * 40_000_000, i as u128 * 40_000_000 + 20_000_000),
                "{n} cue {i} interval"
            );
        }
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for i in 0..3 {
            let at = Duration::from_nanos(i as u64 * 40_000_000 + 5_000_000);
            assert!(video.seek_raw(at).unwrap().is_some());
            assert_eq!(
                fvid::subtitles::active(&cues, at, Duration::ZERO).map(|v| v.text.as_str()),
                Some(wanted_texts[i].as_str()),
                "{n} active at frame {i}"
            );
        }
    }
}

#[test]
fn mp4_tx3g_and_sidecar_bom_prefixed_subtitles_strip_bom_and_preserve_cues() {
    let m = manifest();
    let mp4_case = &m["mp4_case"];
    let n = mp4_case["file"].as_str().unwrap();
    let wanted_texts: Vec<String> = mp4_case["texts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
        Cursor::new(bytes(n)),
        Default::default(),
    )
    .unwrap();
    let cues = r.cues(0).unwrap();
    assert_eq!(cues.len(), 3, "{n}");
    for (i, cue) in cues.iter().enumerate() {
        assert_eq!(cue.text, wanted_texts[i], "{n} cue {i}");
        assert!(!cue.text.contains('\u{feff}'), "{n} cue {i} leaked BOM");
        assert_eq!(
            (cue.start.as_nanos(), cue.end.as_nanos()),
            (i as u128 * 20_000_000, (i as u128 + 1) * 20_000_000),
            "{n} cue {i} interval"
        );
    }
    let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes(n)),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let frame = video.read_frame().unwrap().unwrap();
        let pixels: Vec<_> = frame
            .picture
            .y
            .iter()
            .chain(&frame.picture.cb)
            .chain(&frame.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(pixels, bytes(&format!("frame-{i}.yuv16")));
    }

    // Sidecar WebVTT, SRT, and ASS with leading UTF-8 BOMs:
    let webvtt_bom = "\u{feff}WEBVTT\n\nNOTE 00:00:00.000 --> 00:00:00.500\nhidden note\n\n00:00:01.000 --> 00:00:02.000\n\u{feff}<i>Visible WebVTT</i>\n";
    let vtt_cues = fvid::subtitles::parse(webvtt_bom);
    assert_eq!(
        vtt_cues,
        vec![fvid::subtitles::Cue {
            start: Duration::from_secs(1),
            end: Duration::from_secs(2),
            text: "Visible WebVTT".to_owned(),
        }]
    );

    let srt_bom = "\u{feff}00:00:01,000 --> 00:00:02,000\n\u{feff}First SRT without counter\n\n2\n00:00:03,000 --> 00:00:04,000\n\u{feff}<i></i>\n";
    let srt_cues = fvid::subtitles::parse(srt_bom);
    assert_eq!(
        srt_cues,
        vec![fvid::subtitles::Cue {
            start: Duration::from_secs(1),
            end: Duration::from_secs(2),
            text: "First SRT without counter".to_owned(),
        }]
    );

    let ass_bom = "\u{feff}[Events]\n\u{feff}Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,\u{feff}ASS BOM cue\n";
    let ass_cues = fvid::subtitles::parse(ass_bom);
    assert_eq!(
        ass_cues,
        vec![fvid::subtitles::Cue {
            start: Duration::from_secs(1),
            end: Duration::from_secs(2),
            text: "ASS BOM cue".to_owned(),
        }]
    );
}
