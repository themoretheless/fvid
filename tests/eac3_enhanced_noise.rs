#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{Eac3Decoder, frame},
};
use std::{io::Cursor, path::Path};
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-enhanced-noise")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let v = m["cases"].as_array().unwrap().clone();
    assert_eq!(v.len(), 224);
    v
}
fn raw(c: &serde_json::Value) -> Vec<u8> {
    let b = read(c["file"].as_str().unwrap());
    let h = frame(&b).unwrap();
    assert_eq!((h.blocks, h.channels, h.frame_bytes), (6, 2, 4096));
    for ch in 0..2 {
        assert_eq!(h.aht[ch], c["mask"].as_u64().unwrap() & (1 << ch) != 0);
    }
    assert_eq!(h.aht[5], c["mask"].as_u64().unwrap() & 4 != 0);
    let mut d = Eac3Decoder::new(&[], 48000, 2).unwrap();
    d.decode_complete(&b, 0, 0).unwrap().unwrap().data
}
fn samples(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|x| f32::from_le_bytes(x.try_into().unwrap()))
        .collect()
}
fn flags(c: &serde_json::Value, b: usize, ch: usize) -> bool {
    c["flags"][b % 6][ch].as_u64() == Some(1)
}
fn verify(c: &serde_json::Value, b: &[u8], lead: usize) {
    assert_eq!(b.len(), (3 * 1536 - lead) * 8, "{} size", c["name"]);
    let s = samples(b);
    assert!(s.iter().all(|x| x.is_finite()));
    for block in 1..18 {
        for ch in 0..2 {
            let peak = ((block * 256 - lead)..((block + 1) * 256 - lead))
                .map(|i| s[2 * i + ch].abs())
                .fold(0.0f32, f32::max);
            if !flags(c, block, ch) && !flags(c, block - 1, ch) {
                assert_eq!(
                    peak, 0.0,
                    "{} block{block} ch{ch}: disabled zero-bit dither must be silent after overlap",
                    c["name"]
                );
            } else {
                assert!(
                    peak > 1e-6,
                    "{} block{block} ch{ch}: enabled dither must carry noise",
                    c["name"]
                );
            }
        }
    }
}
fn open(c: &serde_json::Value, mkv: bool) -> Box<dyn AudioStream> {
    let b = Cursor::new(read(
        c[if mkv { "video" } else { "file" }].as_str().unwrap(),
    ));
    if mkv {
        Box::new(fvid::playback_webm_audio::WebmAudioReader::open(b, Default::default()).unwrap())
    } else {
        Box::new(fvid::playback_ac3::Ac3AudioReader::open(b, Default::default()).unwrap())
    }
}
fn packets(s: &mut dyn AudioStream) -> Vec<(std::time::Duration, Vec<u8>)> {
    let mut d = s.make_decoder().unwrap();
    let mut out = vec![];
    while let Some(step) = s.next_step().unwrap() {
        let p = match step {
            AudioStep::Encoded(p) => d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .and_then(|p| s.present_decoded(p.packet, p.source_pts).unwrap()),
            AudioStep::Pcm(p) => Some(p),
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => {
                s.reset_decoder(&mut d).unwrap();
                None
            }
        };
        if let Some(p) = p {
            out.push((p.presentation_time(), p.data));
        }
    }
    out.extend(d.drain_presented(s).into_iter().map(|p| (p.presentation_time(), p.data)));
    s.validate_eof().unwrap();
    out
}
fn acceptance(aht: bool) {
    for c in cases()
        .iter()
        .filter(|c| c["allocated"] == false && (c["mask"].as_u64().unwrap() != 0) == aht)
    {
        verify(c, &raw(c), 256);
        for mkv in [false, true] {
            let mut s = open(c, mkv);
            let duration = s.duration();
            let expected = packets(&mut *s)
                .into_iter()
                .flat_map(|(_, b)| b)
                .collect::<Vec<_>>();
            verify(c, &expected, if mkv { 0 } else { 256 });
            s.rewind();
            let replay = packets(&mut *s)
                .into_iter()
                .flat_map(|(_, b)| b)
                .collect::<Vec<_>>();
            assert!(replay == expected, "{} rewind mkv={mkv}", c["name"]);
            assert_eq!(s.duration(), duration);
        }
    }
}
#[test]
fn conventional_zero_bit_dither_obeys_per_channel_block_flags() {
    acceptance(false);
}
#[test]
fn aht_zero_bit_dither_obeys_per_channel_block_flags() {
    acceptance(true);
}
#[test]
fn zero_bit_noise_scales_with_exponent_and_is_not_shared_between_channels() {
    for c in cases()
        .iter()
        .filter(|c| c["allocated"] == false && c["exponent"] == 8 && c["pattern"] == "on")
    {
        let a = samples(&raw(c));
        let name = c["file"].as_str().unwrap().replace("-e8-", "-e12-");
        let mut d = Eac3Decoder::new(&[], 48000, 2).unwrap();
        let b = samples(&d.decode_complete(&read(&name), 0, 0).unwrap().unwrap().data);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert_eq!(*x, y * 16.0, "{} noise exponent scaling", c["name"]);
        }
        let mut xx = 0.0f64;
        let mut yy = 0.0f64;
        let mut xy = 0.0f64;
        for f in a.chunks_exact(2) {
            let (x, y) = (f64::from(f[0]), f64::from(f[1]));
            xx += x * x;
            yy += y * y;
            xy += x * y;
        }
        assert!(xx > 0.0 && yy > 0.0);
        let correlation = xy / (xx * yy).sqrt();
        assert!(
            correlation.abs() < 0.2,
            "{} shared-noise correlation {correlation}",
            c["name"]
        );
    }
}
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn worker_preserves_zero_bit_noise_and_silence_on_rewind_and_seek() {
    use std::time::Duration;
    for c in cases() {
        for mkv in [false, true] {
            let expected = packets(&mut *open(&c, mkv));
            assert_eq!(expected.len(), 3);
            let seek = Duration::from_millis(20);
            let mut wanted = expected.clone();
            wanted[0].0 = seek;
            wanted[0].1.drain(..960 * 8);
            capture::worker_check_with_layout_and_seek_expected(
                open(&c, mkv),
                &expected,
                seek,
                &wanted,
                3,
            );
        }
    }
}
