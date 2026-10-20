#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[test]
fn signed_aac_packet_windows_seek_without_moving_later_samples() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let raw = std::fs::read(root.join("aac-height-normal-ten.f32le")).unwrap();
    let input: Vec<f32> = raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    for (name, head_packet, head, tail, offset, gap) in [
        ("head", 0, 17, 0, 0, 0),
        ("middle-head", 2, 17, 13, 0, 0),
        ("head-gap", 2, 17, 13, 0, 4768),
        ("offset-head", 0, 17, 13, 2400, 0),
        ("full-head", 0, 1024, 13, 0, 0),
    ] {
        let end = 6144 + offset + gap - tail;
        let mut expected = vec![0.; end * 10];
        for i in 0..6 {
            let first = if i == head_packet { head } else { 0 };
            let last = if i == 5 { 1024 - tail } else { 1024 };
            let at = i * 1024 + offset + if i >= 2 { gap } else { 0 };
            expected[(at + first) * 10..(at + last) * 10]
                .copy_from_slice(&input[(i * 1024 + first) * 10..(i * 1024 + last) * 10]);
        }
        let bytes = std::fs::read(root.join(format!("aac-clock-{name}.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        let mut count = 0;
        while video.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 6);
        for target in [0, 50_000_000, 50_000_001, 150_000_000] {
            let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            reader.seek_to(target);
            let mut decoder = reader.make_decoder().unwrap();
            let mut actual = vec![0f32; end * 10];
            let mut returned_end = 0;
            while let Some(p) = reader.next_packet().unwrap() {
                if let Some(f) = decoder
                    .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                    .unwrap()
                {
                    if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                        assert!(out.pts >= target as u64, "{name}/{target}");
                        let at =
                            ((u128::from(out.pts) * 48000 + 500_000_000) / 1_000_000_000) as usize;
                        let samples: Vec<f32> = out
                            .data
                            .chunks_exact(4)
                            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                            .collect();
                        returned_end = at + samples.len() / 10;
                        assert!(returned_end <= end);
                        actual[at * 10..returned_end * 10].copy_from_slice(&samples);
                    }
                }
            }
            let begin = ((target as u128 * 48000).div_ceil(1_000_000_000) as usize).min(end);
            if begin < end {
                assert_eq!(returned_end, end, "{name}/{target}");
            }
            for (a, b) in actual[begin * 10..].iter().zip(&expected[begin * 10..]) {
                assert!((a - b).abs() < 1e-6, "{name}/{target}: {a}/{b}");
            }
            assert_eq!(
                reader.duration(),
                Some(std::time::Duration::from_nanos(
                    end as u64 * 1_000_000_000 / 48000
                )),
                "{name}/{target}"
            );
            reader.rewind();
            assert_eq!(
                reader.duration(),
                Some(std::time::Duration::from_nanos(
                    end as u64 * 1_000_000_000 / 48000
                ))
            );
        }
    }
}
