#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-compact-sizes")
            .join(name),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn check(c: &serde_json::Value) {
    let name = c["file"].as_str().unwrap();
    let mut a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
        .unwrap_or_else(|e| panic!("root {name}: {e}"));
    let mut b =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
            .unwrap_or_else(|e| panic!("owned {name}: {e}"));
    for i in [2, 0, 1] {
        let mut x = Vec::new();
        let mut y = Vec::new();
        a.read_packet(0, i, &mut x).unwrap();
        b.read_packet(0, i, &mut y).unwrap();
        assert_eq!(x, bytes(&format!("frame-{i}.packet")));
        assert_eq!(y, x);
    }
    let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes(name)),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let f = video.read_frame().unwrap().unwrap();
        assert_eq!((f.presentation_time.ticks, f.duration.ticks), (i, 1));
        let p: Vec<_> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(p, bytes(&format!("frame-{i}.yuv16")), "{name}");
    }
    assert!(video.read_frame().unwrap().is_none());
    if c["target"] == "text" {
        for i in 0..c["texts"].as_array().unwrap().len() {
            let mut x = Vec::new();
            let mut y = Vec::new();
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            assert_eq!(
                x,
                bytes(&format!(
                    "text-{}-{}-{i}.packet",
                    c["width"],
                    c["texts"].as_array().unwrap().len()
                ))
            );
            assert_eq!(y, x);
            assert_eq!(
                a.tracks()[1].samples.get(i).unwrap().size as u64,
                c["sizes"][i].as_u64().unwrap()
            );
        }
        let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        assert_eq!(cues.len(), c["texts"].as_array().unwrap().len());
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        let scale = c["timescale"].as_u64().unwrap();
        for (i, cue) in cues.iter().enumerate() {
            let ns = i as u64 * 1_000_000_000 / scale;
            assert_eq!(cue.text, c["texts"][i].as_str().unwrap());
            assert_eq!(cue.start, Duration::from_nanos(ns));
            assert_eq!(
                cue.end,
                Duration::from_nanos((i as u64 + 1) * 1_000_000_000 / scale)
            );
            assert!(v.seek_raw(cue.start).unwrap().is_some());
            let frame = u128::from(ns / 20_000_000);
            assert_eq!(v.frame_interval(), Some((frame, frame + 1, 50)));
        }
    }
}
#[test]
fn ordinary_size_table_controls_preserve_packets_playback_and_caption_seek() {
    for c in cases().into_iter().filter(|c| c["mode"] == "stsz") {
        check(&c);
    }
}
#[test]
fn compact_size_tables_preserve_packets_playback_and_caption_seek() {
    for c in cases().into_iter().filter(|c| c["mode"] == "stz2") {
        check(&c);
    }
}

#[test]
fn uniform_pcm_keeps_chunk_budget_exact_samples_decode_seek_and_eof() {
    let pcm = bytes("pcm.s16le");
    let expected = bytes("pcm.f32le");
    for c in cases().into_iter().filter(|c| c["target"] == "pcm") {
        let name = c["file"].as_str().unwrap();
        let mut limits = fvid::container::mp4::Limits::default();
        limits.samples = 4;
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(name)), limits).unwrap();
        let mut limits = fvid_media::owned_mp4::Limits::default();
        limits.samples = 4;
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(name)), limits).unwrap();
        assert_eq!(
            a.tracks().len(),
            2,
            "{name}: 3 video + 1 audio index records"
        );
        assert_eq!(b.tracks().len(), 2);
        assert_eq!(
            (a.tracks()[1].samples.len(), a.tracks()[1].samples.cost()),
            (2880, 1)
        );
        assert_eq!(
            (b.tracks()[1].samples.len(), b.tracks()[1].samples.cost()),
            (2880, 1)
        );
        for i in [0, 2879, 1, 960] {
            let mut x = Vec::new();
            let mut y = Vec::new();
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            assert_eq!(x, pcm[i * 2..i * 2 + 2], "{name}");
            assert_eq!(y, x);
        }
        a.restrict_packet_bytes(1);
        assert!(
            a.read_packet(1, 0, &mut Vec::new())
                .unwrap_err()
                .to_string()
                .contains("packet exceeds budget")
        );
        let mut audio = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap();
        let mut decoder = audio.make_decoder().unwrap();
        for start in [0, 960, 1920, 0] {
            assert_eq!(audio.seek(start), start);
            decoder.reset();
            let mut actual = Vec::new();
            let mut sample = start;
            while let Some(p) = audio.read_packet().unwrap() {
                assert_eq!((p.pts, p.duration), (sample, 1));
                let decoded = decoder
                    .decode_encoded(&p.data, p.pts as u64, p.duration as u64)
                    .unwrap()
                    .unwrap();
                actual.extend(decoded.data);
                sample += 1;
            }
            assert_eq!(
                actual,
                expected[start as usize * 4..],
                "{name} PCM suffix at {start}"
            );
            assert!(audio.read_packet().unwrap().is_none());
        }
        audio.rewind();
        assert_eq!(audio.read_packet().unwrap().unwrap().data, pcm[..2]);
    }
}

#[test]
fn malformed_or_ambiguous_size_tables_refuse_before_playback() {
    let doc: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    for c in doc["malformed"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let damage = c["damage"].as_str().unwrap();
        let expected = match damage {
            "both" => "expected exactly one MP4 sample-size table",
            "missing" => "missing MP4 stsz box",
            "reserved" | "flags" => "reserved MP4 compact sample-size fields",
            "version" => "version",
            "zero-entry" => "zero MP4 sample size",
            s if s.starts_with("field-") => "invalid MP4 compact sample-size field width",
            _ => "invalid sample-size table",
        };
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
            .err()
            .expect("damaged root input accepted");
        let b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
                .err()
                .expect("damaged owned input accepted");
        assert!(matches!(a, fvid::Error::Invalid(_)));
        assert!(!b.is_unsupported());
        assert!(a.to_string().contains(expected), "{name}: {a}");
        assert!(b.to_string().contains(expected), "{name}: {b}");
    }
}

#[test]
fn compact_mace_preserves_block_units_exact_pcm_and_eof() {
    let mut reference = None;
    for name in ["mace-stsz-4.mp4", "mace-stz2-4.mp4"] {
        let mut limits = fvid::container::mp4::Limits::default();
        limits.samples = 4;
        let demux =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(name)), limits).unwrap();
        assert_eq!(demux.tracks().len(), 2);
        assert_eq!(
            (
                demux.tracks()[1].samples.len(),
                demux.tracks()[1].samples.cost()
            ),
            (480, 1)
        );
        let mut audio = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap();
        let mut decoder = audio.make_decoder().unwrap();
        let mut pcm = Vec::new();
        for i in 0..480 {
            let p = audio.read_packet().unwrap().unwrap();
            assert_eq!((p.pts, p.duration), (i * 6, 6));
            assert_eq!(p.data, [0, 0]);
            pcm.extend(
                decoder
                    .decode_encoded(&p.data, p.pts as u64, p.duration as u64)
                    .unwrap()
                    .unwrap()
                    .data,
            );
        }
        assert_eq!(pcm.len(), 2880 * 4);
        assert!(audio.read_packet().unwrap().is_none());
        if let Some(expected) = &reference {
            assert_eq!(&pcm, expected);
        } else {
            reference = Some(pcm);
        }
    }
}

#[test]
fn compact_empty_init_tables_populate_real_fragments_and_keep_edit_seek_clocks() {
    let doc: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    for name in doc["fragmented"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
    {
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
                .unwrap();
        assert_eq!(a.tracks()[1].samples.len(), 3);
        assert_eq!(b.tracks()[1].samples.len(), 3);
        for i in 0..3 {
            let mut x = Vec::new();
            let mut y = Vec::new();
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            assert_eq!(x, bytes(&format!("fragment-text-{i}.packet")));
            assert_eq!(y, x);
            let s = a.tracks()[1].samples.get(i).unwrap();
            assert_eq!((s.pts, s.duration), (i as i64 * 40, 40));
        }
        let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(name)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let f = video.read_frame().unwrap().unwrap();
            assert_eq!((f.presentation_time.ticks, f.duration.ticks), (i * 40, 40));
            let p: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(p, bytes(&format!("frame-{i}.yuv16")));
        }
        assert!(video.read_frame().unwrap().is_none());
        let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        assert_eq!(
            cues.iter()
                .map(|c| (c.text.as_str(), c.start.as_nanos(), c.end.as_nanos()))
                .collect::<Vec<_>>(),
            [
                ("Middle caption", 0, 40_000_000),
                ("Last caption", 40_000_000, 80_000_000)
            ]
        );
        let mut native =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        for ns in [
            0,
            39_999_999,
            40_000_000,
            79_999_999,
            80_000_000,
            119_999_999,
        ] {
            let at = Duration::from_nanos(ns);
            assert!(native.seek_raw(at).unwrap().is_some());
            let (s, e, scale) = native.frame_interval().unwrap();
            let frame = u128::from(ns / 40_000_000);
            assert_eq!(
                (
                    s * 1_000_000_000 / u128::from(scale),
                    e * 1_000_000_000 / u128::from(scale)
                ),
                (frame * 40_000_000, (frame + 1) * 40_000_000)
            );
            assert_eq!(
                fvid::subtitles::active(&cues, at, Duration::ZERO).map(|c| c.text.as_str()),
                if ns < 40_000_000 {
                    Some("Middle caption")
                } else if ns < 80_000_000 {
                    Some("Last caption")
                } else {
                    None
                }
            );
        }
    }
}
