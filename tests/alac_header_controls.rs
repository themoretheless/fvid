#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[test]
fn reserved_alac_element_fields_refuse_before_body_decoding() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-header-controls");
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in m["invalid"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let cookie =
            std::fs::read(root.join(format!("{}.cookie", c["control"].as_str().unwrap()))).unwrap();
        let channels = u16::from(cookie[9]);
        let packet = std::fs::read(root.join(format!("{name}.packet"))).unwrap();
        let bytes = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..2 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        let e = fvid::codec::alac_decoder::AlacDecoder::packet_samples(
            &cookie, 48000, channels, &packet,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains(c["error"].as_str().unwrap()), "{name}: {e}");
        let e =
            fvid_media::owned_alac::AlacDecoder::packet_samples(&cookie, 48000, channels, &packet)
                .unwrap_err()
                .to_string();
        assert!(e.contains(c["error"].as_str().unwrap()), "{name}: {e}");
        let mut decoder =
            fvid::codec::alac_decoder::AlacDecoder::new(&cookie, 48000, channels).unwrap();
        let e = decoder.decode_pcm(&packet).unwrap_err().to_string();
        assert!(e.contains(c["error"].as_str().unwrap()), "{name}: {e}");
        let mut decoder =
            fvid_media::owned_alac::AlacDecoder::new(&cookie, 48000, channels).unwrap();
        let e = decoder.decode_pcm(&packet).unwrap_err().to_string();
        assert!(e.contains(c["error"].as_str().unwrap()), "{name}: {e}");
        let e = match fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        ) {
            Ok(_) => panic!("{name}: invalid header accepted"),
            Err(e) => e.to_string(),
        };
        assert!(e.contains(c["error"].as_str().unwrap()), "{name}: {e}");
    }
}
#[test]
fn unchanged_header_controls_preserve_exact_pcm_and_seek() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-header-controls");
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in m["controls"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let depth = c["depth"].as_u64().unwrap();
        let compressed = c["compressed"].as_bool().unwrap();
        let mut expected = Vec::new();
        for n in 0..32 {
            for ch in 0..channels {
                let v = (((ch + 1) * 101 + n * 7) as i32) * if ch % 2 == 0 { 1 } else { -1 };
                let v = if compressed {
                    v * 256 + ch as i32 + 1
                } else {
                    v
                };
                expected.extend((v as f32 / (1u64 << (depth - 1)) as f32).to_le_bytes());
            }
        }
        for extension in ["mkv", "m4a"] {
            let bytes = std::fs::read(root.join(format!("{name}.{extension}"))).unwrap();
            for first in [0, 11usize] {
                let mut reader: Box<dyn AudioStream> = if extension == "mkv" {
                    Box::new(
                        fvid::playback_webm_audio::WebmAudioReader::open(
                            Cursor::new(bytes.clone()),
                            Default::default(),
                        )
                        .unwrap(),
                    )
                } else {
                    Box::new(
                        fvid::playback_mp4_audio::Mp4AudioReader::open(
                            Cursor::new(bytes.clone()),
                            Default::default(),
                        )
                        .unwrap(),
                    )
                };
                reader.seek_to(if extension == "mkv" && first != 0 {
                    208334
                } else {
                    first as i64
                });
                let mut decoder = reader.make_decoder().unwrap();
                let mut pcm = Vec::new();
                while let Some(p) = reader.next_packet().unwrap() {
                    let f = decoder
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                        pcm.extend(out.data);
                    }
                }
                assert_eq!(
                    pcm,
                    expected[first * channels * 4..],
                    "{name}/{extension}/{first}"
                );
            }
        }
    }
}
