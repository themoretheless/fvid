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
            .join("tests/fixtures/playback-errors/eac3-aht")
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
            AudioStep::ResetDecoder => stream.reset_decoder(decoder).unwrap(),
        }
    }
    out.extend(decoder.drain_presented(stream).into_iter().flat_map(|p| p.data));
    stream.validate_eof().unwrap();
    out
}
#[test]
fn active_aht_scalar_and_mixed_planes_preserve_pcm_and_clocks() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 8);
    let mut cases: Vec<_> = manifest["cases"].as_array().unwrap().iter().collect();
    cases.sort_by_key(|c| c["mask"].as_u64().unwrap() == 0);
    for c in cases {
        let blocks = c["blocks"].as_u64().unwrap() as usize;
        let raw = read(c["file"].as_str().unwrap());
        let h = frame(&raw).unwrap_or_else(|| panic!("{}: AHT header refused", c["name"]));
        assert_eq!(
            (h.sample_rate, h.channels, h.blocks, h.frame_bytes),
            (48000, 2, blocks, 4096)
        );
        assert_eq!(h.aht[0], c["mask"].as_u64().unwrap() & 1 != 0);
        assert_eq!(h.aht[1], c["mask"].as_u64().unwrap() & 2 != 0);
        let control = read(c["control"].as_str().unwrap());
        let mut reference = Eac3Decoder::new(&[], 48000, 2).unwrap();
        let expected = reference
            .decode_complete(&control, 0, 0)
            .unwrap()
            .unwrap()
            .data;
        let mut decoder = Eac3Decoder::new(&[], 48000, 2).unwrap();
        let got = decoder.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        assert_eq!(expected.len(), (3 * blocks * 256 - 256) * 8);
        assert!(
            expected
                .chunks_exact(4)
                .any(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() > 0.001)
        );
        assert!(got == expected, "{} raw PCM", c["name"]);
        for container in [false, true] {
            let open = |converted: bool| -> Box<dyn AudioStream> {
                let key = match (container, converted) {
                    (false, false) => "control",
                    (false, true) => "file",
                    (true, false) => "control_video",
                    (true, true) => "video",
                };
                let bytes = Cursor::new(read(c[key].as_str().unwrap()));
                if container {
                    Box::new(
                        fvid::playback_webm_audio::WebmAudioReader::open(bytes, Default::default())
                            .unwrap(),
                    )
                } else {
                    Box::new(
                        fvid::playback_ac3::Ac3AudioReader::open(bytes, Default::default())
                            .unwrap(),
                    )
                }
            };
            let mut control = open(false);
            let mut d = control.make_decoder().unwrap();
            let expected = drain(&mut *control, &mut d);
            let mut stream = open(true);
            let mut d = stream.make_decoder().unwrap();
            assert!(
                drain(&mut *stream, &mut d) == expected,
                "{} container={container}",
                c["name"]
            );
            assert_eq!(stream.duration(), control.duration());
            stream.rewind();
            d.reset();
            assert!(
                drain(&mut *stream, &mut d) == expected,
                "{} rewind container={container}",
                c["name"]
            );
        }
    }
}
