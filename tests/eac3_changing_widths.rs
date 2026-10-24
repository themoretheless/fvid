#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{Eac3Decoder, transient::apply_window},
};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-changing-widths")
            .join(name),
    )
    .unwrap()
}
fn open(name: &str) -> fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>> {
    fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(read(name)), Default::default())
        .unwrap()
}
fn packets(s: &mut dyn AudioStream) -> Vec<(Duration, Vec<u8>)> {
    let mut d = s.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(step) = s.next_step().unwrap() {
        if let AudioStep::Encoded(p) = step {
            if let Some(p) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(p) = s.present_decoded(p.packet, p.source_pts).unwrap() {
                    out.push((p.presentation_time(), p.data));
                }
            }
        }
    }
    out.extend(
        d.drain_presented(s)
            .into_iter()
            .map(|p| (p.presentation_time(), p.data)),
    );
    s.validate_eof().unwrap();
    out
}
fn pcm(s: &mut dyn AudioStream) -> Vec<u8> {
    packets(s).into_iter().flat_map(|(_, p)| p).collect()
}
fn expected(c: &serde_json::Value, correct: bool, changing: bool) -> Vec<u8> {
    let frame = c["blocks"].as_u64().unwrap() as usize * 256;
    let count = frame * 3;
    let mut mixed = vec![0.0f32; count * 8];
    for component in c["components"].as_array().unwrap() {
        let bytes = pcm(&mut open(component["control"].as_str().unwrap()));
        let control: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let width = component["coded_to_native"].as_array().unwrap().len();
        assert_eq!(control.len(), count * width);
        for coded in 0..width {
            let native = component["coded_to_native"][coded].as_u64().unwrap() as usize;
            let mut plane = vec![0.0; 1536 + count.max(2 * frame + 960)];
            for n in 0..count {
                plane[1536 + n] =
                    if coded < component["widths"][n / frame].as_u64().unwrap() as usize {
                        control[n * width + native]
                    } else {
                        0.0
                    };
            }
            let w = &component["windows"][coded];
            if correct && !w.is_null() {
                for f in 0..3 {
                    apply_window(
                        &mut plane,
                        -1536,
                        (f * frame) as i64 + w["location"].as_i64().unwrap(),
                        w["length"].as_u64().unwrap() as u8,
                    )
                    .unwrap();
                }
            }
            for n in 0..count {
                let roles = &component["roles"][if changing { n / frame } else { 0 }];
                if coded >= roles.as_array().unwrap().len() {
                    continue;
                }
                let role = &roles[coded];
                let slot = c["speakers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|s| s == role)
                    .unwrap();
                mixed[n * 8 + slot] = plane[1536 + n];
            }
        }
    }
    mixed
        .iter()
        .flat_map(|v| v.clamp(-1.0, 1.0).to_le_bytes())
        .collect()
}
#[test]
fn changing_coded_widths_preserve_active_planes_and_clear_returning_overlap() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(manifest["accepted"], true);
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 32);
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let want = expected(c, true, true);
        if c["blocks"] == 3 {
            assert_ne!(
                want,
                expected(c, true, false),
                "mapping witness must distinguish stale speaker assignment"
            );
        }
        if c["transient"].as_bool().unwrap() {
            assert_ne!(
                want,
                expected(c, false, true),
                "transient must alter PCM across mapping boundary"
            );
        }
        let raw = read(&format!("{name}.ec3"));
        let rate = c["rate"].as_u64().unwrap() as u32;
        let mut d = Eac3Decoder::new(&[], rate, 8).unwrap();
        let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        assert!(
            got == want[256 * 8 * 4..],
            "{name} raw: first differing float {:?}",
            got.chunks_exact(4)
                .zip(want[256 * 8 * 4..].chunks_exact(4))
                .position(|(a, b)| a != b)
        );
        d.reset();
        assert_eq!(
            d.decode_complete(&raw, 0, 0).unwrap().unwrap().data,
            got,
            "{name} reset"
        );
        let mut s = open(&format!("{name}.mkv"));
        assert_eq!(pcm(&mut s), want, "{name} split Matroska");
        s.rewind();
        assert_eq!(pcm(&mut s), want, "{name} rewind");
        let got = packets(&mut open(&format!("{name}.mkv")));
        assert_eq!(got.len(), 3);
        let samples = c["blocks"].as_u64().unwrap() * 256;
        for (i, (pts, data)) in got.iter().enumerate() {
            assert_eq!(
                *pts,
                Duration::from_nanos(i as u64 * samples * 1_000_000_000 / u64::from(rate)),
                "{name} PTS"
            );
            assert_eq!(data.len(), samples as usize * 8 * 4);
        }
        let seek = Duration::from_millis(20);
        let seek_sample = u64::from(rate) / 50;
        let index = (seek_sample / samples) as usize;
        let mut wanted = got[index..].to_vec();
        wanted[0].0 = seek;
        wanted[0]
            .1
            .drain(..((seek_sample % samples) as usize * 8 * 4));
        capture::worker_check_with_layout_and_seek_expected(
            Box::new(open(&format!("{name}.mkv"))),
            &got,
            seek,
            &wanted,
            0x63f,
        );
    }
}

#[test]
fn six_block_mp4_samples_preserve_changing_widths_pcm_clocks_and_seek_history() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let cases: Vec<_> = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mp4"].is_string())
        .collect();
    assert_eq!(cases.len(), 12);
    for c in cases {
        let open_mp4 = || -> Box<dyn AudioStream> {
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(read(c["mp4"].as_str().unwrap())),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        let mut s = open_mp4();
        assert_eq!(s.channels(), 8);
        assert_eq!(s.speaker_mask().unwrap(), Some(0x63f));
        let got = packets(&mut *s);
        assert_eq!(got.len(), 3);
        assert_eq!(
            got.iter()
                .flat_map(|(_, b)| b.iter().copied())
                .collect::<Vec<_>>(),
            expected(c, true, true)
        );
        let rate = c["rate"].as_u64().unwrap();
        for (i, (pts, data)) in got.iter().enumerate() {
            assert_eq!(
                *pts,
                Duration::from_nanos(i as u64 * 1536 * 1_000_000_000 / rate)
            );
            assert_eq!(data.len(), 1536 * 8 * 4);
        }
        s.rewind();
        assert_eq!(packets(&mut *s), got);
        let seek = Duration::from_millis(40);
        let sample = rate / 25;
        let index = (sample / 1536) as usize;
        let mut wanted = got[index..].to_vec();
        wanted[0].0 = seek;
        wanted[0].1.drain(..((sample % 1536) as usize * 8 * 4));
        capture::worker_check_with_layout_and_seek_expected(open_mp4(), &got, seek, &wanted, 0x63f);
    }
}
