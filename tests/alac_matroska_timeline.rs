#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[test]
fn alac_sample_clock_preserves_container_origin_real_gaps_and_exact_seek() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/alac-timeline");
    for (name, gap, delay, tail) in [
        ("offset", 0, 0, 0),
        ("offset-gap", 48, 0, 0),
        ("offset-delay-tail", 0, 17, 13),
        ("offset-discard-last", 0, 0, 128),
    ] {
        let size = if gap == 0 { 128 } else { 96 };
        let bytes = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..4 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        for target in [0, 51_000_000, 51_000_001, 56_000_001, 1_000_000_000] {
            let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            let initial = Some(std::time::Duration::from_nanos(
                (2400 - delay + 4 * size + gap - tail) as u64 * 1_000_000_000 / 48000,
            ));
            assert_eq!(reader.duration(), initial, "{name} before decode");
            assert_eq!(reader.seek_to(target), target);
            let mut decoder = reader.make_decoder().unwrap();
            let mut decoded_packets = 0;
            let mut actual = Vec::new();
            let mut times = Vec::new();
            while let Some(p) = reader.next_packet().unwrap() {
                decoded_packets += 1;
                let f = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap();
                if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                    actual.extend(out.data);
                    times.push(out.pts);
                }
            }
            let floor = (target as u128 * 48000).div_ceil(1_000_000_000) as usize;
            let mut expected = Vec::new();
            let mut first_packet = None;
            let mut stamps = Vec::new();
            for i in 0..4 {
                let at = 2400 - delay + i * size + if i >= 2 { gap } else { 0 };
                let first = floor.saturating_sub(at).min(size);
                let last = if i == 3 { size - tail } else { size };
                if first < last {
                    first_packet.get_or_insert(i);
                    stamps.push((at + first) as u64 * 1_000_000_000 / 48000);
                    for n in i * size + first..i * size + last {
                        expected.extend(
                            ((n * 119 % 24001) as f32 / 32768. - 12000. / 32768.).to_le_bytes(),
                        );
                    }
                }
            }
            assert_eq!(times, stamps, "{name}/{target}");
            assert_eq!(actual, expected, "{name}/{target}");
            assert_eq!(
                decoded_packets,
                first_packet.map_or(0, |i| 4 - i),
                "{name}/{target} unnecessary PCM preroll"
            );
            let duration = Some(std::time::Duration::from_nanos(
                (2400 - delay + 4 * size + gap - tail) as u64 * 1_000_000_000 / 48000,
            ));
            assert_eq!(reader.duration(), duration, "{name}/{target} EOF duration");
            reader.rewind();
            assert_eq!(reader.duration(), duration, "{name} rewind duration");
            reader.seek_to(1);
            assert_eq!(reader.duration(), duration, "{name} seek duration");
        }
    }
}
