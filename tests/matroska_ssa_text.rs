#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/matroska-ssa-text")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn mapped_script_packets_and_actual_pictures_are_preserved() {
    let mut reference = None;
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut a =
            fvid::container::webm::WebmReader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        a.scan_all().unwrap();
        b.scan_all().unwrap();
        assert_eq!(
            a.tracks[1].codec,
            format!("S_TEXT/{}", c["codec"].as_str().unwrap())
        );
        let ai: Vec<_> = a
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == 2)
            .map(|(i, _)| i)
            .collect();
        let bi: Vec<_> = b
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == 2)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(ai.len(), 3);
        assert_eq!(bi.len(), 3);
        for (i, (&x, &y)) in ai.iter().zip(&bi).enumerate() {
            assert_eq!(
                (a.packets[x].pts_ns, a.packets[x].duration_ns),
                (i as i64 * 40_000_000, Some(20_000_000))
            );
            assert_eq!(
                (b.packets[y].pts_ns, b.packets[y].duration_ns),
                (a.packets[x].pts_ns, a.packets[x].duration_ns)
            );
            let wanted = bytes(&format!("{n}-{i}.packet"));
            assert_eq!(a.read_packet(x).unwrap(), wanted);
            assert_eq!(b.read_packet(y).unwrap(), wanted);
        }
        let mut v =
            fvid::playback_webm::WebmVideoReader::open_software(Cursor::new(bytes(n)), 32 << 20)
                .unwrap();
        let mut pictures = Vec::new();
        for i in 0..3 {
            assert!(v.read_frame().unwrap());
            assert_eq!(v.dimensions(), [16, 16]);
            assert_eq!(
                v.frame_interval().map(|v| (v.0, v.1)),
                Some((i * 40_000_000, (i + 1) * 40_000_000))
            );
            pictures.push(v.rgb().to_vec());
        }
        assert!(!v.read_frame().unwrap());
        if let Some(ref r) = reference {
            assert_eq!(&pictures, r);
        } else {
            assert_ne!(pictures[0], pictures[1]);
            assert_ne!(pictures[1], pictures[2]);
            reference = Some(pictures);
        }
    }
}
#[test]
fn canonical_ssa_empty_layer_is_metadata_instead_of_visible_text() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = fvid::playback_webm_subtitles::WebmSubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        assert_eq!(cues.len(), 3);
        for (i, cue) in cues.iter().enumerate() {
            assert_eq!(cue.text, c["text"][i].as_str().unwrap(), "{n}");
            assert_eq!(
                (cue.start.as_nanos(), cue.end.as_nanos()),
                (i as u128 * 40_000_000, i as u128 * 40_000_000 + 20_000_000)
            );
        }
    }
}
#[test]
fn seek_displays_only_ssa_text_with_exact_exclusive_endpoints() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = fvid::playback_webm_subtitles::WebmSubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for ns in [
            0,
            19_999_999,
            20_000_000,
            40_000_000,
            59_999_999,
            60_000_000,
            80_000_000,
            99_999_999,
            100_000_000,
            110_000_000,
        ] {
            let at = Duration::from_nanos(ns);
            assert!(v.seek_raw(at).unwrap().is_some());
            let wanted = if ns % 40_000_000 < 20_000_000 {
                Some(c["text"][(ns / 40_000_000) as usize].as_str().unwrap())
            } else {
                None
            };
            assert_eq!(
                fvid::subtitles::active(&cues, at, Duration::ZERO).map(|cue| cue.text.as_str()),
                wanted,
                "{n} at{ns}"
            );
        }
    }
}
