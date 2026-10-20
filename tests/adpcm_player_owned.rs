#![cfg(feature = "player")]
use fvid::audio::{AudioStep, AudioStream};
use std::io::Cursor;
#[test]
fn owned_adpcm_player_keeps_edits_repeats_seek_and_pcm() {
    let ramp: Vec<f32> = (1..=64).map(|n| n as f32 / 32768.).collect();
    let ima_stereo: Vec<f32> = (0..9)
        .flat_map(|n| [n as f32 / 32768., -128. / 32768.])
        .collect();
    let ms_stereo: Vec<f32> = [-16., 0., 16., 32., 48., 64.]
        .into_iter()
        .flat_map(|v| [v / 32768., -128. / 32768.])
        .collect();
    for (name, channels, source) in [
        ("ima4-ramp-edits.mov", 1, ramp),
        ("ima-wav-stereo-edits.mov", 2, ima_stereo),
        ("ms-adpcm-stereo-edits.mov", 2, ms_stereo),
    ] {
        for name in [name.to_string(), name.replace(".mov", "-video.mov")] {
            let expected: Vec<u8> = vec![0.; 2 * channels]
                .iter()
                .chain(&source)
                .chain(&source[2 * channels..])
                .flat_map(|v| v.to_le_bytes())
                .collect();
            let frames = expected.len() / (channels * 4);
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(&name);
            if name.ends_with("-video.mov") {
                let video = fvid_media::decode_video(&path).unwrap();
                assert!(video.video_frames > 0, "{name}: no video decoded");
                assert_eq!((video.width, video.height), (32, 32));
            }
            let bytes = std::fs::read(path).unwrap();
            let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(bytes),
                Default::default(),
            )
            .unwrap();
            assert_eq!(
                stream.duration().unwrap().as_nanos(),
                frames as u128 * 1_000_000_000 / 48000,
                "{name}"
            );
            for target in [0, 1, 2, 3, frames - 1, frames, 0] {
                assert_eq!(stream.seek_to(target as i64), target as i64, "{name}");
                let mut decoder = stream.make_decoder().unwrap();
                let mut actual = Vec::new();
                let mut cursor = target as u64;
                while let Some(step) = stream.next_step().unwrap() {
                    let pcm = match step {
                        AudioStep::ResetDecoder => {
                            stream.reset_decoder(&mut decoder).unwrap();
                            continue;
                        }
                        AudioStep::Pcm(pcm) => Some(pcm),
                        AudioStep::Encoded(packet) => decoder
                            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                            .unwrap()
                            .and_then(|v| stream.present_decoded(v.packet, v.source_pts).unwrap()),
                    };
                    if let Some(pcm) = pcm {
                        assert_eq!(pcm.pts, cursor, "{name}");
                        cursor += pcm.data.len() as u64 / (channels as u64 * 4);
                        actual.extend(pcm.data);
                    }
                }
                stream.validate_eof().unwrap();
                assert_eq!(cursor, frames as u64, "{name}");
                assert!(
                    actual == expected[target * channels * 4..],
                    "{name} target {target}: PCM mismatch"
                );
            }
        }
    }
}
