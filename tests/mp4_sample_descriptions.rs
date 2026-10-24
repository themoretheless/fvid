use fvid::container::mp4::Mp4Reader;
use std::{io::Cursor, path::PathBuf};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-sample-descriptions")
            .join(format!("{name}.mp4")),
    )
    .unwrap()
}
#[test]
fn regular_and_fragmented_samples_retain_their_own_descriptions() {
    for label in ["equivalent", "changing"] {
        for suffix in ["", "-fragmented"] {
            let name = format!("{label}{suffix}");
            let mut reader =
                Mp4Reader::open_indexed(Cursor::new(bytes(&name)), Default::default()).unwrap();
            let track = &reader.tracks()[0];
            assert_eq!(track.sample_descriptions.len(), 2);
            assert_eq!(track.samples.len(), 3);
            for (i, expected) in [1, 2, 1].into_iter().enumerate() {
                let sample = track.samples.get(i).unwrap();
                assert_eq!(sample.description_index, expected, "{name}/{i}");
                assert_eq!(
                    (sample.dts, sample.pts, sample.duration),
                    (i as u64, i as i64, 1)
                );
                let setup = track.description_for_sample(i).unwrap();
                assert_eq!(setup.codec, *b"vp09");
                assert_eq!(
                    (setup.width, setup.height),
                    (
                        if label == "changing" && i == 1 {
                            32
                        } else {
                            16
                        },
                        16
                    )
                );
            }
            assert!(track.description_for_sample(3).is_none());
            // Both init and fragmented addressing must select the actual frame bytes.
            for i in 0..3 {
                let mut packet = Vec::new();
                reader.read_packet(0, i, &mut packet).unwrap();
                assert!(!packet.is_empty());
                let expected = std::fs::read(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/playback-errors/mp4-sample-descriptions")
                        .join(format!("{label}-frame-{i}.vp9")),
                )
                .unwrap();
                assert_eq!(packet, expected, "{name}/{i}");
            }
        }
    }
}
#[test]
fn compact_index_retains_chunk_descriptions_without_expanding_pcm_frames() {
    let mut reader =
        Mp4Reader::open_indexed(Cursor::new(bytes("compact-pcm")), Default::default()).unwrap();
    let track = &reader.tracks()[0];
    assert!(matches!(
        track.samples,
        fvid::container::mp4::SampleIndex::Uniform(_)
    ));
    assert_eq!(track.samples.len(), 6);
    assert_eq!(track.samples.cost(), 3);
    for (i, id) in [1, 1, 2, 2, 1, 1].into_iter().enumerate() {
        assert_eq!(track.samples.get(i).unwrap().description_index, id);
        let setup = track.description_for_sample(i).unwrap();
        assert_eq!(
            (setup.channels, setup.bit_depth),
            if id == 2 { (1, 32) } else { (2, 16) }
        );
    }
    for i in 0..6 {
        let mut packet = Vec::new();
        reader.read_packet(0, i, &mut packet).unwrap();
        assert_eq!(
            packet,
            (i * 4..i * 4 + 4).map(|n| n as u8).collect::<Vec<_>>()
        );
    }
}
#[test]
fn every_used_description_and_later_entry_is_validated() {
    for label in ["equivalent", "changing"] {
        for suffix in ["-chunk-0", "-chunk-3", "-fragment-3", "-external-second"] {
            let name = format!("{label}{suffix}");
            let error = Mp4Reader::open_indexed(Cursor::new(bytes(&name)), Default::default())
                .err()
                .expect("invalid description must refuse");
            let message = error.to_string();
            assert!(
                message.contains(if suffix == "-fragment-3" {
                    "fragment sample description index"
                } else if suffix == "-external-second" {
                    "invalid sample data reference"
                } else {
                    "sample-to-chunk mapping"
                }),
                "{name}: {message}"
            );
        }
    }
}
#[cfg(feature = "player")]
mod playback {
    use super::*;
    use fvid::playback_native::{NativeReader, RawFrame};
    use std::time::Duration;
    fn check(frame: RawFrame, width: usize) {
        match frame {
            RawFrame::Planar(p) => {
                assert_eq!((p.frame.width, p.frame.height), (width, 16));
                assert_eq!(p.frame.data, vec![128; (width * 16 * 3 / 2) as usize]);
            }
            _ => panic!("VP9 must preserve owned planar output"),
        }
    }
    #[test]
    fn changing_geometry_payloads_are_valid_against_independent_single_entry_controls() {
        for i in 0..3 {
            let mut reader = NativeReader::software(
                Cursor::new(bytes(&format!("changing-control-{i}"))),
                32 << 20,
            )
            .unwrap();
            check(
                reader.read_frame_raw().unwrap().unwrap(),
                if i == 1 { 32 } else { 16 },
            );
            assert!(reader.read_frame_raw().unwrap().is_none());
        }
    }
    #[test]
    fn equivalent_descriptions_keep_video_seek_rewind_and_eof() {
        for name in ["equivalent", "equivalent-fragmented"] {
            let mut reader = NativeReader::software(Cursor::new(bytes(name)), 32 << 20).unwrap();
            for i in 0..3 {
                check(reader.read_frame_raw().unwrap().unwrap(), 16);
                assert_eq!(reader.frame_interval(), Some((i, i + 1, 50)));
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
            check(reader.read_frame_raw().unwrap().unwrap(), 16);
            for ms in [0, 20, 40, 59] {
                check(
                    reader.seek_raw(Duration::from_millis(ms)).unwrap().unwrap(),
                    16,
                );
            }
        }
    }
    #[test]
    fn preindexed_video_reconfigures_while_audio_changes_remain_explicit() {
        let indexed =
            Mp4Reader::open_indexed(Cursor::new(bytes("changing")), Default::default()).unwrap();
        let mut video =
            fvid::playback_mp4::Mp4VideoReader::from_demuxer(indexed, 0, 32 << 20).unwrap();
        for width in [16, 32, 16] {
            assert_eq!(
                video.read_frame().unwrap().unwrap().picture.dimensions(),
                (width, 16)
            );
        }
        let indexed =
            Mp4Reader::open_indexed(Cursor::new(bytes("compact-pcm")), Default::default()).unwrap();
        let error = fvid::playback_mp4_audio::Mp4AudioReader::from_demuxer(indexed, 0)
            .err()
            .expect("audio constructor must validate indexed setup changes");
        assert!(
            error
                .to_string()
                .contains("sample description changes require decoder reconfiguration"),
            "{error}"
        );
    }
    #[test]
    fn changing_descriptions_present_each_coded_geometry() {
        for name in ["changing", "changing-fragmented"] {
            let mut reader = NativeReader::software(Cursor::new(bytes(name)), 32 << 20).unwrap();
            for width in [16, 32, 16] {
                check(reader.read_frame_raw().unwrap().unwrap(), width);
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
            check(reader.read_frame_raw().unwrap().unwrap(), 16);
            for ms in [0, 20, 40, 21, 59, 1] {
                check(
                    reader.seek_raw(Duration::from_millis(ms)).unwrap().unwrap(),
                    if (20..40).contains(&ms) { 32 } else { 16 },
                );
                assert_eq!(
                    reader.frame_interval(),
                    Some((u128::from(ms / 20), u128::from(ms / 20 + 1), 50))
                );
            }
        }
    }
}
