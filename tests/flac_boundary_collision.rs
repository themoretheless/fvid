#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(bytes: Vec<u8>) -> fvid::playback_flac::FlacAudioReader {
    fvid::playback_flac::FlacAudioReader::open(Cursor::new(bytes), Default::default()).unwrap()
}
#[test]
fn flac_pcm_bytes_cannot_impersonate_metadata_or_frame_boundaries() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-boundary-collision");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let bytes = std::fs::read(base.join(name)).unwrap();
        let pcm = std::fs::read(base.join(case["pcm"].as_str().unwrap())).unwrap();
        let mut decoder =
            fvid::codec::flac_decoder::FlacDecoder::new(&bytes[8..42], 44100, 1).unwrap();
        let mut expected = Vec::new();
        for (i, packet) in case["packets"].as_array().unwrap().iter().enumerate() {
            let packet = std::fs::read(base.join(packet.as_str().unwrap())).unwrap();
            let decoded = decoder
                .decode_packet(&packet, (i * 64) as i64, 64)
                .unwrap()
                .unwrap();
            assert_eq!(
                decoded.packet.data,
                pcm[i * 256..i * 256 + 256],
                "{name}: fullpacketvalidity"
            );
            expected.push((decoded.packet.presentation_time(), decoded.packet.data));
        }
        let source = match fvid::playback_flac::FlacAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        ) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{name}: validFLACindexrefused{e}"));
                continue;
            }
        };
        let actual = source
            .flac()
            .frames
            .iter()
            .map(|f| f.size)
            .collect::<Vec<_>>();
        let wanted = case["packets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                std::fs::metadata(base.join(p.as_str().unwrap()))
                    .unwrap()
                    .len() as usize
            })
            .collect::<Vec<_>>();
        if actual != wanted {
            failures.push(format!(
                "{name}: indexsplitPCMbytes{actual:?}instead of{wanted:?}"
            ));
        }
        let end = source.duration().unwrap();
        for target in [expected[1].0, end] {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                capture::worker_check_with_seek(
                    Box::new(open(bytes.clone())),
                    &expected,
                    Some(target),
                )
            }))
            .is_err()
            {
                failures.push(format!("{name}: actualplaybackrefusesvalidPCM"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
