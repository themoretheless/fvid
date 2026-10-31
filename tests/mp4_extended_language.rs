#![cfg(feature = "player")]
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn path(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-extended-language")
        .join(n)
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(path(n)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn metadata(c: &serde_json::Value) {
    let n = c["file"].as_str().unwrap();
    let expected = c["expected"].as_str().unwrap();
    let r =
        fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    let o =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    assert_eq!(r.tracks()[0].language, expected, "root {n}");
    assert_eq!(o.tracks()[0].language, expected, "owned {n}");
    let info = fvid_media::probe(&path(n)).unwrap();
    assert_eq!(
        info.streams[0].metadata.get("language").map(String::as_str),
        if expected.is_empty() {
            None
        } else {
            Some(expected)
        },
        "probe {n}"
    );
}
#[test]
fn absent_and_invalid_extended_tags_keep_the_legacy_language() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "fallback")
    {
        metadata(c);
    }
}
#[test]
fn extended_language_preserves_region_script_extensions_and_private_use() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "valid")
    {
        metadata(c);
    }
}
#[test]
fn all_language_cases_preserve_actual_video_packets_pictures_and_seek() {
    for c in manifest()["cases"].as_array().unwrap() {
        let n = c["file"].as_str().unwrap();
        let mut r =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        for i in 0..3 {
            let mut p = Vec::new();
            r.read_packet(0, i, &mut p).unwrap();
            assert_eq!(p, bytes(&format!("frame-{i}.packet")));
            let s = r.tracks()[0].samples.get(i).unwrap();
            assert_eq!((s.pts, s.duration), (i as i64, 1));
        }
        let mut d = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(n)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let f = d.read_frame().unwrap().unwrap();
            let p: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|x| x.to_le_bytes())
                .collect();
            assert_eq!(p, bytes(&format!("frame-{i}.yuv16")));
        }
        assert!(d.read_frame().unwrap().is_none());
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let rgb: Vec<_> = (0..3)
            .map(|_| {
                v.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap()
            })
            .collect();
        assert!(v.read_frame_raw().unwrap().is_none());
        v.rewind().unwrap();
        for ms in [0, 19, 20, 39, 40, 59, 60] {
            let i = (ms / 20).min(2) as usize;
            assert_eq!(
                v.seek_raw(Duration::from_millis(ms))
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                rgb[i]
            );
        }
    }
}
#[test]
fn extended_tags_survive_owned_mp4_to_matroska_copy() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "valid" && c["copy_refusal"] != true)
    {
        let n = c["copy_file"].as_str().unwrap();
        let expected = c["expected"].as_str().unwrap();
        let mut r =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut out = Cursor::new(Vec::new());
        let stats = fvid_media::owned_mp4_matroska::write(&mut r, &mut out, None, None).unwrap();
        assert_eq!(stats.packets, 2);
        let mut root =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut root_out = Cursor::new(Vec::new());
        fvid::container::mp4_matroska::write(&mut root, &mut root_out, None, None).unwrap();
        assert_eq!(root_out.get_ref(), out.get_ref());
        let mut webm = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(out.into_inner()),
            Default::default(),
        )
        .unwrap();
        webm.scan_all().unwrap();
        let mut source_video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let mut copy_video = fvid::playback_native::NativeReader::software(
            Cursor::new(root_out.into_inner()),
            32 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            assert_eq!(
                source_video
                    .read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                copy_video
                    .read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap()
            );
        }
        assert!(source_video.read_frame_raw().unwrap().is_none());
        assert!(copy_video.read_frame_raw().unwrap().is_none());

        assert_eq!(
            webm.track_languages.get(&1).map(String::as_str),
            Some(if expected.is_empty() { "und" } else { expected }),
            "copy {n}"
        );
        assert_eq!(
            webm.track_legacy_languages.get(&1).map(String::as_str),
            Some("und"),
            "extended tags use LanguageBCP47, not legacy Language: {n}"
        );
        for i in 0..2 {
            assert_eq!(webm.packets[i].pts_ns, i as i64 * 20_000_000);
            assert_eq!(
                webm.read_packet(i).unwrap(),
                bytes(&format!("avc-frame-{i}.packet"))
            );
        }
    }
}

#[test]
fn oversized_copy_metadata_refuses_without_truncating_the_source_tag() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["copy_refusal"] == true)
    {
        metadata(c);
        let n = c["copy_file"].as_str().unwrap();
        let mut r =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut out = Cursor::new(Vec::new());
        let err = fvid_media::owned_mp4_matroska::write(&mut r, &mut out, None, None)
            .err()
            .expect("oversized metadata must be explicitly refused")
            .to_string();
        assert!(
            err.contains("Matroska language tag exceeds limit"),
            "{n}: {err}"
        );
        assert!(out.into_inner().is_empty());
        let mut r =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut out = Cursor::new(Vec::new());
        let err = fvid::container::mp4_matroska::write(&mut r, &mut out, None, None)
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("Matroska language tag exceeds limit"));
        assert!(out.into_inner().is_empty());
    }
}

#[test]
fn independent_matroska_ietf_tags_obey_precedence_and_explicit_bound() {
    let mut reference = fvid::playback_native::NativeReader::software(
        Cursor::new(bytes("v0-missing.mp4")),
        32 << 20,
    )
    .unwrap();
    let rgb: Vec<_> = (0..3)
        .map(|_| {
            reference
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap()
        })
        .collect();
    for c in manifest()["matroska_cases"].as_array().unwrap() {
        let n = c["file"].as_str().unwrap();
        if c["refusal"] == true {
            let a =
                fvid::container::webm::WebmReader::open(Cursor::new(bytes(n)), Default::default())
                    .err()
                    .unwrap()
                    .to_string();
            let b =
                fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes(n)), Default::default())
                    .err()
                    .unwrap()
                    .to_string();
            assert!(a.contains("EBML value exceeds limit"));
            assert_eq!(a, b);
            continue;
        }
        let a = fvid::container::webm::WebmReader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap_or_else(|e| panic!("root {n}: {e}"));
        let b = fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap_or_else(|e| panic!("owned {n}: {e}"));
        for language in [a.track_languages.get(&1), b.track_languages.get(&1)] {
            assert_eq!(language.map(String::as_str), c["expected"].as_str());
        }
        assert_eq!(
            a.track_legacy_languages.get(&1).map(String::as_str),
            Some("eng")
        );
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for expected in &rgb {
            assert_eq!(
                v.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                *expected
            );
        }
        assert!(v.read_frame_raw().unwrap().is_none());
        v.rewind().unwrap();
        assert_eq!(
            v.seek_raw(Duration::from_millis(39))
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            rgb[1]
        );
    }
}

#[test]
fn cstring_terminator_separates_language_from_atom_padding() {
    let m = manifest();
    for version in [0, 1] {
        for label in ["en-US", "padded-zero", "padded-nontext", "trailing-payload"] {
            let n = format!("v{version}-{label}.mp4");
            let c = m["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["file"] == n)
                .unwrap();
            metadata(c);
        }
    }
}

#[path = "support/audio_transitions.rs"]
mod transitions;
#[test]
fn extended_language_survives_audio_and_subtitle_tracks_without_changing_media() {
    use fvid::audio::AudioStream;
    let mut reference =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes("v0-en-US.mp4")), 32 << 20)
            .unwrap();
    let rgb: Vec<_> = (0..3)
        .map(|_| {
            reference
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap()
        })
        .collect();
    for c in manifest()["muxed_cases"].as_array().unwrap() {
        let n = c["file"].as_str().unwrap();
        let expected = c["expected"].as_str().unwrap();
        let r = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        let o = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        assert_eq!(r.tracks().len(), 2);
        assert_eq!(o.tracks().len(), 2);
        for t in r.tracks() {
            assert_eq!(t.language, expected);
        }
        for t in o.tracks() {
            assert_eq!(t.language, expected);
        }
        let info = fvid_media::probe(&path(n)).unwrap();
        assert_eq!(info.streams.len(), 2);
        for s in info.streams {
            assert_eq!(
                s.metadata.get("language").map(String::as_str),
                Some(expected)
            );
        }
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for color in &rgb {
            assert_eq!(
                v.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                *color
            );
        }
        assert!(v.read_frame_raw().unwrap().is_none());
        v.rewind().unwrap();
        assert_eq!(
            v.seek_raw(Duration::from_millis(39))
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            rgb[1]
        );
        if c["kind"] == "pcm" {
            let mut a = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(bytes(n)),
                Default::default(),
            )
            .unwrap();
            let decoded: Vec<_> = transitions::collect(&mut a)
                .into_iter()
                .flat_map(|(_, _, _, data)| data)
                .collect();
            assert_eq!(decoded, bytes("pcm.f32le"));
            a.rewind();
            let again: Vec<_> = transitions::collect(&mut a)
                .into_iter()
                .flat_map(|(_, _, _, data)| data)
                .collect();
            assert_eq!(again, decoded);
        } else {
            let mut s = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
                Cursor::new(bytes(n)),
                Default::default(),
            )
            .unwrap();
            let cues = s.cues(0).unwrap();
            let actual: Vec<_> = cues
                .iter()
                .map(|c| (c.text.as_str(), c.start.as_nanos(), c.end.as_nanos()))
                .collect();
            assert_eq!(
                actual,
                [
                    ("A", 0, 20_000_000),
                    ("BBB", 20_000_000, 40_000_000),
                    ("CC", 40_000_000, 60_000_000)
                ]
            );
        }
    }
}
