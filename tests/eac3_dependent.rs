#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{DependentDecoder, Eac3Decoder, dependent_frame, frame},
};
use std::{io::Cursor, path::Path};
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-dependent")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let c = m["cases"].as_array().unwrap().clone();
    assert_eq!(c.len(), 496);
    c
}
fn control(c: &serde_json::Value) -> Vec<f32> {
    let width = c["channels"].as_u64().unwrap() as u16;
    Eac3Decoder::new(&[], 48000, width)
        .unwrap()
        .decode_complete(&read(c["control"].as_str().unwrap()), 0, 0)
        .unwrap()
        .unwrap()
        .data
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn verify(c: &serde_json::Value, got: &[u8], expected: &[f32]) {
    let width = c["channels"].as_u64().unwrap() as usize;
    assert_eq!(got.len(), expected.len() * 4);
    let mut max = 0.0f32;
    let mut peak = vec![0.0f32; width];
    for (i, b) in got.chunks_exact(4).enumerate() {
        let sample = f32::from_le_bytes(b.try_into().unwrap());
        assert!(sample.is_finite());
        let coded = i % width;
        let native = c["order"][coded].as_u64().unwrap() as usize;
        let target = expected[i / width * width + native];
        max = max.max((sample - target).abs());
        peak[coded] = peak[coded].max(sample.abs());
    }
    assert!(
        peak.iter().all(|&p| p > 1e-6),
        "{} muted plane {peak:?}",
        c["name"]
    );
    let bound = if c["exact"] == true {
        0.0
    } else {
        c["quantization_bound"].as_f64().unwrap() + 1e-7
    };
    assert!(
        f64::from(max) <= bound,
        "{} max error {max} bound {bound}",
        c["name"]
    );
}
#[test]
fn dependent_reconstruction_preserves_coded_channel_order_and_normative_map() {
    for c in cases() {
        let raw = read(c["file"].as_str().unwrap());
        let h = dependent_frame(&raw).unwrap();
        assert_eq!(h.substream_id, c["identity"].as_u64().unwrap() as u8);
        assert_eq!(h.channel_map, c["channel_map"].as_u64().map(|v| v as u16));
        assert_eq!(
            usize::from(h.frame.channels),
            c["channels"].as_u64().unwrap() as usize
        );
        assert!(
            frame(&raw).is_none(),
            "base parser must not reinterpret a dependent stream"
        );
        let expected = control(&c);
        let mut decoder = DependentDecoder::new(48000, h.frame.channels, h.substream_id).unwrap();
        assert_eq!(decoder.spec().channels, h.frame.channels);
        let pcm = decoder.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        verify(&c, &pcm, &expected);
        decoder.reset();
        assert!(
            decoder.decode_complete(&raw, 0, 0).unwrap().unwrap().data == pcm,
            "{} reset",
            c["name"]
        );
    }
}
#[test]
fn video_extraction_retains_dependent_payload_and_continuous_state() {
    for c in cases() {
        let width = c["channels"].as_u64().unwrap() as u16;
        let identity = c["identity"].as_u64().unwrap() as u8;
        let mut source = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(read(c["video"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        let mut decoder = DependentDecoder::new(48000, width, identity).unwrap();
        let mut combined = vec![];
        let mut count = 0;
        while let Some(step) = source.next_step().unwrap() {
            if let AudioStep::Encoded(p) = step {
                let base = frame(&p.data).unwrap();
                assert_eq!(base.frame_bytes, 4096);
                let offset = 4096 * (usize::from(identity) + 1);
                let bytes = &p.data[offset..offset + 4096];
                assert_eq!(dependent_frame(bytes).unwrap().substream_id, identity);
                if let Some(p)=decoder.decode_encoded(bytes,0,0).unwrap() {combined.extend(p.data);}
                count += 1;
            }
        }
        while let Some(p)=decoder.finish_packet().unwrap() {combined.extend(p.packet.data);}
        assert_eq!(count, 3);
        source.validate_eof().unwrap();
        verify(&c, &combined, &control(&c));
    }
}
#[test]
fn inconsistent_channel_maps_and_wrong_stream_identity_are_refused() {
    let bad = read("bad-map-count.ec3");
    let error = dependent_frame(&bad).unwrap_err();
    assert!(
        error.to_string().contains("channel map disagrees"),
        "{error}"
    );
    let c = cases().into_iter().find(|c| c["identity"] == 7).unwrap();
    let mut decoder =
        DependentDecoder::new(48000, c["channels"].as_u64().unwrap() as u16, 0).unwrap();
    let error = decoder
        .decode_complete(&read(c["file"].as_str().unwrap()), 0, 0)
        .err()
        .expect("identity mismatch must fail");
    assert!(
        error
            .to_string()
            .contains("substream ID or coded channel count changed"),
        "{error}"
    );
    assert!(DependentDecoder::new(48000, 2, 8).is_err());
    assert!(DependentDecoder::new(48000, 8, 0).is_err());
    let mut source = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(read("bad-map-count.mkv")),
        Default::default(),
    )
    .unwrap();
    let Some(AudioStep::Encoded(packet)) = source.next_step().unwrap() else {
        panic!("missing synthetic audio");
    };
    let error = dependent_frame(&packet.data[4096..]).unwrap_err();
    assert!(
        error.to_string().contains("channel map disagrees"),
        "{error}"
    );
    assert!(
        dependent_frame(&read(c["control"].as_str().unwrap())).is_err(),
        "independent frames must not enter dependent reconstruction"
    );
}
