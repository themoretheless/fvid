#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{Eac3Decoder, frame},
};
use std::{io::Cursor, path::Path};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-bsid")
            .join(name),
    )
    .unwrap()
}
fn drain(stream: &mut dyn AudioStream, decoder: &mut Box<dyn AudioDecode>) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(step) = stream.next_step().unwrap() {
        match step {
            AudioStep::Encoded(p) => {
                if let Some(d) = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                {
                    if let Some(p) = stream.present_decoded(d.packet, d.source_pts).unwrap() {
                        out.extend(p.data);
                    }
                }
            }
            AudioStep::Pcm(p) => out.extend(p.data),
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => stream.reset_decoder(decoder).unwrap(),
        }
    }
    out.extend(decoder.drain_presented(stream).into_iter().flat_map(|p| p.data));
    stream.validate_eof().unwrap();
    out
}
#[test]
fn compatible_bsid_preserves_exact_pcm_in_elementary_and_matroska_playback() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 36);
    for c in m["cases"].as_array().unwrap() {
        let control = read(c["control"].as_str().unwrap());
        let mut d = Eac3Decoder::new(&[], 48000, 2).unwrap();
        let expected = d.decode_complete(&control, 0, 0).unwrap().unwrap().data;
        assert_eq!(expected.len(), (3 * 1536 - 256) * 8);
        assert!(
            expected
                .chunks_exact(4)
                .any(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() > 0.001)
        );
        let raw = read(c["file"].as_str().unwrap());
        for chunk in raw.chunks_exact(1024) {
            assert_eq!(chunk[5] >> 3, c["bsid"].as_u64().unwrap() as u8);
            let h = frame(chunk).unwrap_or_else(|| panic!("{}: missing header", c["name"]));
            assert_eq!(
                (h.sample_rate, h.channels, h.blocks, h.frame_bytes),
                (48000, 2, 6, 1024)
            );
        }
        for container in [false, true] {
            // Container decoding retains codec delay for the container clock to
            // present; elementary decoding trims its historical 256-sample delay.
            let expected = if container {
                let name = c["control"]
                    .as_str()
                    .unwrap()
                    .replace("-control.ec3", "-bsid16.mkv");
                let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(read(&name)),
                    Default::default(),
                )
                .unwrap();
                let mut decoder = stream.make_decoder().unwrap();
                let pcm = drain(&mut stream, &mut decoder);
                assert_eq!(pcm.len(), 3 * 1536 * 8);
                assert!(pcm[256 * 8..] == expected);
                pcm
            } else {
                expected.clone()
            };
            let mut stream: Box<dyn AudioStream> = if container {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(read(c["video"].as_str().unwrap())),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_ac3::Ac3AudioReader::open(
                        Cursor::new(raw.clone()),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            assert_eq!((stream.sample_rate(), stream.channels()), (48000, 2));
            let mut d = stream.make_decoder().unwrap();
            let got = drain(&mut *stream, &mut d);
            assert!(
                got == expected,
                "{} container={container}: got {} expected {} bytes",
                c["name"],
                got.len(),
                expected.len()
            );
            stream.rewind();
            d.reset();
            assert!(
                drain(&mut *stream, &mut d) == expected,
                "{} rewind",
                c["name"]
            );
        }
    }
}
#[test]
fn reserved_and_incompatible_identification_stays_rejected() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for c in m["invalid"].as_array().unwrap() {
        let raw = read(c["file"].as_str().unwrap());
        assert!(frame(&raw).is_none());
        let mut d = Eac3Decoder::new(&[], 48000, 2).unwrap();
        let e = d
            .decode_complete(&raw, 0, 0)
            .err()
            .expect("invalid bsid accepted");
        assert!(e.to_string().contains("bsid"), "{e}");
    }
}
