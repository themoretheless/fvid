#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::codec::eac3_decoder::Eac3Decoder;
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/eac3-stereo-downmix")
}
fn decode(name: &str, width: u16, stereo: bool) -> Vec<f32> {
    let data = std::fs::read(root().join(name)).unwrap();
    let mut d = if stereo {
        Eac3Decoder::new_stereo(&[], 48000)
    } else {
        Eac3Decoder::new(&[], 48000, width)
    }
    .unwrap()
    .with_stereo_downmix(fvid::codec::eac3_decoder::StereoDownmixPolicy {
        mode: fvid::codec::eac3_decoder::StereoDownmixMode::LtRt,
        headroom: false,
    });
    d.decode_complete(&data, 0, 2304)
        .unwrap()
        .unwrap()
        .data
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
#[test]
fn requested_eac3_ltrt_uses_its_own_metadata_and_surround_polarity() {
    let native = decode("l7-f1-control.ec3", 6, false);
    assert_eq!(native.len(), 2048 * 6);
    assert!(native.iter().any(|v| v.abs() > 1e-4));
    let actual = decode("l7-f1-c0-s0.ec3", 2, true);
    let mut error = 0f32;
    for i in 0..512 {
        let n = &native[i * 6..i * 6 + 6];
        let l = (n[0] + 1.414 * n[2] - 0.841 * n[4] - 0.841 * n[5]).clamp(-1., 1.);
        let r = (n[1] + 1.414 * n[2] + 0.841 * n[4] + 0.841 * n[5]).clamp(-1., 1.);
        error = error
            .max((actual[i * 2] - l).abs())
            .max((actual[i * 2 + 1] - r).abs());
    }
    assert!(error < 4e-7, "requested EAC3 Lt/Rt error {error}");
}
use fvid::codec::eac3_decoder::{StereoDownmixMode as Mode, StereoDownmixPolicy as Policy};
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn values(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn decode_policy(name: &str, width: u16, stereo: bool, policy: Policy) -> Vec<f32> {
    let data = std::fs::read(root().join(name)).unwrap();
    let mut d = if stereo {
        Eac3Decoder::new_stereo(&[], 48000)
    } else {
        Eac3Decoder::new(&[], 48000, width)
    }
    .unwrap()
    .with_stereo_downmix(policy);
    values(&d.decode_complete(&data, 0, 2304).unwrap().unwrap().data)
}
fn oracle(c: &serde_json::Value, native: &[f32], policies: [Policy; 3]) -> Vec<f32> {
    let centre = [1.414f32, 1.189, 1., 0.841, 0.707, 0.595, 0.5, 0.];
    let surround = [0.841f32, 0.841, 0.841, 0.841, 0.707, 0.595, 0.5, 0.];
    let layout = c["layout"].as_u64().unwrap();
    let lfe = c["lfe"].as_bool().unwrap();
    let width = c["channels"].as_u64().unwrap() as usize;
    let ci = match layout {
        1 => Some(0),
        3 | 5 | 7 => Some(2),
        _ => None,
    };
    let li = if !lfe {
        None
    } else {
        Some(match layout {
            1 => 1,
            0 | 2 | 4 | 6 => 2,
            _ => 3,
        })
    };
    let shift = usize::from(lfe);
    let rear: Vec<usize> = match layout {
        4 => vec![2 + shift],
        5 => vec![3 + shift],
        6 => vec![2 + shift, 3 + shift],
        7 => vec![3 + shift, 4 + shift],
        _ => vec![],
    };
    let mut out = Vec::new();
    let mut offset = 0;
    for (block, frames) in [512, 768, 768].into_iter().enumerate() {
        let policy = policies[block];
        let mix = &c["mixes"][block];
        let stated = !mix.is_null();
        let ltrt = match policy.mode {
            Mode::LoRo => false,
            Mode::LtRt => true,
            Mode::Automatic => layout > 2 && stated && mix[0] == 1,
        };
        let cg = if stated && matches!(layout, 3 | 5 | 7) {
            centre[mix[if ltrt { 1 } else { 3 }].as_u64().unwrap() as usize]
        } else {
            0.707
        };
        let sg = if stated && layout & 4 != 0 {
            surround[mix[if ltrt { 2 } else { 4 }].as_u64().unwrap() as usize]
        } else {
            0.707
        };
        let lg = c["lfe_codes"][block]
            .as_u64()
            .map_or(0., |code| 10f64.powf((5.5 - code as f64) / 20.) as f32);
        let rw = if !ltrt && rear.len() == 1 {
            0.7 * sg
        } else {
            sg
        };
        let sum = if layout == 1 { 0. } else { 1. }
            + if ci.is_some() { cg } else { 0. }
            + if ltrt {
                sg * rear.len() as f32
            } else if rear.is_empty() {
                0.
            } else {
                rw
            }
            + lg;
        let scale = if policy.headroom {
            1. / sum.max(1.)
        } else {
            1.
        };
        for i in offset..offset + frames {
            let n = &native[i * width..(i + 1) * width];
            let common = ci.map_or(0., |j| cg * n[j]) + li.map_or(0., |j| lg * n[j]);
            let (left, right) = if layout == 1 { (0., 0.) } else { (n[0], n[1]) };
            let (ls, rs) = if ltrt {
                let s = rear.iter().map(|&j| sg * n[j]).sum::<f32>();
                (-s, s)
            } else {
                match rear.as_slice() {
                    [] => (0., 0.),
                    [j] => (rw * n[*j], rw * n[*j]),
                    [l, r] => (sg * n[*l], sg * n[*r]),
                    _ => unreachable!(),
                }
            };
            out.extend([
                ((left + common + ls) * scale).clamp(-1., 1.),
                ((right + common + rs) * scale).clamp(-1., 1.),
            ]);
        }
        offset += frames;
    }
    out
}
fn close(a: &[f32], b: &[f32], label: &str) {
    assert_eq!(a.len(), b.len(), "{label}");
    let error = a
        .iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(error < 4e-7, "{label}: matrix error {error}");
}
#[test]
fn all_modes_headroom_and_lfe_codes_match_independent_native_pcm_matrix() {
    for c in manifest()["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let width = c["channels"].as_u64().unwrap() as u16;
        let native = decode_policy(
            c["control"].as_str().unwrap(),
            width,
            false,
            Policy::default(),
        );
        assert_eq!(native.len(), 2048 * width as usize);
        for ch in 0..width as usize {
            assert!(
                native
                    .chunks_exact(width as usize)
                    .any(|n| n[ch].abs() > 1e-5),
                "silent source: {name} plane{ch}"
            );
        }
        for mode in [Mode::LoRo, Mode::LtRt, Mode::Automatic] {
            for headroom in [false, true] {
                let policy = Policy { mode, headroom };
                let actual = decode_policy(&format!("{name}.ec3"), 2, true, policy);
                close(
                    &actual,
                    &oracle(c, &native, [policy; 3]),
                    &format!("{name}: {policy:?}"),
                );
            }
        }
    }
}
#[test]
fn checked_headers_retain_ltrt_loro_and_preference_fields() {
    for c in manifest()["cases"].as_array().unwrap() {
        let raw =
            std::fs::read(root().join(format!("{}.ec3", c["name"].as_str().unwrap()))).unwrap();
        let layout = c["layout"].as_u64().unwrap();
        for (i, p) in raw.chunks_exact(4096).enumerate() {
            let f = fvid::codec::eac3_decoder::checked_frame(p).unwrap();
            let mix = &c["mixes"][i];
            if mix.is_null() {
                assert!(f.downmix.is_none());
                continue;
            }
            let d = f.downmix.unwrap();
            let code = |i: usize| Some(mix[i].as_u64().unwrap() as u8);
            assert_eq!(d.preferred, if layout > 2 { code(0) } else { None });
            assert_eq!(
                (d.ltrt_centre, d.centre),
                if matches!(layout, 3 | 5 | 7) {
                    (code(1), code(3))
                } else {
                    (None, None)
                }
            );
            assert_eq!(
                (d.ltrt_surround, d.surround),
                if layout & 4 != 0 {
                    (code(2), code(4))
                } else {
                    (None, None)
                }
            );
            assert_eq!(d.lfe, c["lfe_codes"][i].as_u64().map(|v| v as u8));
        }
    }
}
#[test]
fn midstream_policy_captures_delayed_pcm_and_survives_reset() {
    use fvid::audio::AudioDecode;
    for name in ["l7-f1-c0-s1", "l1-f1-c0-s7", "l5-f0-c3-s4"] {
        let m = manifest();
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let width = c["channels"].as_u64().unwrap() as u16;
        let native = decode_policy(
            c["control"].as_str().unwrap(),
            width,
            false,
            Policy::default(),
        );
        let raw = std::fs::read(root().join(format!("{name}.ec3"))).unwrap();
        let mut d = Eac3Decoder::new_stereo(&[], 48000).unwrap();
        let mut actual = Vec::new();
        let policies = [
            Policy {
                mode: Mode::LtRt,
                headroom: false,
            },
            Policy {
                mode: Mode::Automatic,
                headroom: true,
            },
            Policy {
                mode: Mode::LoRo,
                headroom: true,
            },
        ];
        for (i, p) in raw.chunks_exact(4096).enumerate() {
            d.set_eac3_stereo_downmix(policies[i]);
            if let Some(p) = d.decode_packet(p, (i * 768) as i64, 768).unwrap() {
                actual.extend(values(&p.packet.data));
            }
        }
        while let Some(p) = d.finish_packet().unwrap() {
            actual.extend(values(&p.packet.data));
        }
        close(&actual, &oracle(c, &native, policies), name);
        d.reset();
        let repeat = values(&d.decode_complete(&raw, 0, 2304).unwrap().unwrap().data);
        close(
            &repeat,
            &oracle(c, &native, [policies[2]; 3]),
            "reset retained EAC3 policy",
        );
    }
}
#[test]
fn stereo_policy_preserves_native_and_mono_output() {
    for name in ["l7-f1-c0-s1", "l1-f1-c0-s7", "l0-f0-c7-s0", "l2-f1-c4-s4"] {
        let m = manifest();
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        for width in [1, c["channels"].as_u64().unwrap() as u16] {
            let original = decode_policy(&format!("{name}.ec3"), width, false, Policy::default());
            for mode in [Mode::LoRo, Mode::LtRt, Mode::Automatic] {
                assert_eq!(
                    decode_policy(
                        &format!("{name}.ec3"),
                        width,
                        false,
                        Policy {
                            mode,
                            headroom: true
                        }
                    ),
                    original
                );
            }
        }
    }
}

#[path = "support/audio_capture.rs"]
mod capture;
struct StereoOutput {
    packets: Vec<Vec<u8>>,
    index: usize,
    rate: u32,
    floor: std::cell::Cell<u64>,
}
impl fvid::audio::AudioStream for StereoOutput {
    fn codec(&self) -> &str {
        "A_EAC3"
    }
    fn timescale(&self) -> u32 {
        self.rate
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> u16 {
        2
    }
    fn extra_data(&self) -> &[u8] {
        &[]
    }
    fn audio_tracks(&self) -> Vec<fvid::audio::AudioTrack> {
        vec![]
    }
    fn speaker_mask(&self) -> fvid::Result<Option<u32>> {
        Ok(Some(3))
    }
    fn duration(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_nanos(
            2048 * 1_000_000_000 / self.rate as u64,
        ))
    }
    fn make_decoder(&self) -> fvid::Result<Box<dyn fvid::audio::AudioDecode>> {
        Ok(Box::new(Eac3Decoder::new_stereo(&[], self.rate)?))
    }
    fn reset_decoder(
        &mut self,
        decoder: &mut Box<dyn fvid::audio::AudioDecode>,
    ) -> fvid::Result<()> {
        // Deliberately rebuild with default folding: the real worker must restore
        // its session policy after navigation replaces the decoder instance.
        *decoder = self.make_decoder()?;
        Ok(())
    }
    fn next_packet(&mut self) -> fvid::Result<Option<fvid::audio::EncodedPacket>> {
        let Some(data) = self.packets.get(self.index) else {
            return Ok(None);
        };
        let out = fvid::audio::EncodedPacket {
            data: data.clone(),
            pts: (self.index * 768) as i64,
            duration: 768,
        };
        self.index += 1;
        Ok(Some(out))
    }
    fn present_decoded(
        &self,
        mut p: fvid::audio::AudioPacket,
        source: i64,
    ) -> fvid::Result<Option<fvid::audio::AudioPacket>> {
        assert_eq!(p.data.len() % 8, 0);
        let start = (source - 256).max(0) as u64;
        let frames = p.data.len() / 8;
        let skip = self.floor.get().saturating_sub(start).min(frames as u64) as usize;
        if skip == frames {
            return Ok(None);
        }
        p.data.drain(..skip * 8);
        p.pts = start + skip as u64;
        p.timebase_num = 1;
        p.timebase_den = self.rate;
        Ok(Some(p))
    }
    fn rewind(&mut self) {
        self.index = 0;
        self.floor.set(0);
    }
    fn seek_to(&mut self, at: i64) -> i64 {
        let at = (at.max(0) as u64).min(2048);
        self.floor.set(at);
        self.index = if at == 2048 { 3 } else { 0 };
        at as i64
    }
}
fn worker_source(c: &serde_json::Value) -> StereoOutput {
    use std::io::Cursor;
    let bytes = std::fs::read(root().join(format!("{}.mkv", c["name"].as_str().unwrap()))).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 16 << 20)
            .unwrap();
    let mut count = 0;
    while video.read_frame_raw().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 3);
    let mut demux =
        fvid::container::webm::WebmReader::open(Cursor::new(bytes), Default::default()).unwrap();
    demux.scan_all().unwrap();
    let indices: Vec<_> = demux
        .packets
        .iter()
        .enumerate()
        .filter_map(|(i, p)| (p.track == 2).then_some(i))
        .collect();
    let packets: Vec<_> = indices
        .into_iter()
        .map(|i| demux.read_packet(i).unwrap())
        .collect();
    let raw = std::fs::read(root().join(format!("{}.ec3", c["name"].as_str().unwrap()))).unwrap();
    assert!(packets.concat() == raw);
    assert_eq!(packets.len(), 3);
    StereoOutput {
        packets,
        index: 0,
        rate: c["rate"].as_u64().unwrap() as u32,
        floor: std::cell::Cell::new(0),
    }
}
#[test]
fn eac3_stereo_worker_restores_session_policy_after_decoder_replacement() {
    for name in ["l7-f1-c0-s1", "l1-f1-c0-s7", "l5-f0-c3-s4"] {
        let m = manifest();
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let rate = 48000u32;
        let ch = c["channels"].as_u64().unwrap() as u16;
        let native = decode_policy(c["control"].as_str().unwrap(), ch, false, Policy::default());
        for mode in [Mode::LoRo, Mode::LtRt, Mode::Automatic] {
            let policy = Policy {
                mode,
                headroom: true,
            };
            let expected = oracle(c, &native, [policy; 3]);
            // Use verified decoder bytes for exact worker comparison, after the
            // independently computed scalar PCM has accepted the matrix.
            let actual = decode_policy(&format!("{name}.ec3"), 2, true, policy);
            close(&actual, &expected, name);
            let to_bytes = |x: &[f32]| x.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
            let mut offset = 0;
            let mut full = Vec::new();
            let mut sought = Vec::new();
            let seek_sample = (u64::from(rate) * 20 / 1000) as usize;
            for frames in [512, 768, 768] {
                let time = |n: usize| {
                    std::time::Duration::from_nanos(n as u64 * 1_000_000_000 / u64::from(rate))
                };
                full.push((
                    time(offset),
                    to_bytes(&actual[offset * 2..(offset + frames) * 2]),
                ));
                let first = offset.max(seek_sample);
                if first < offset + frames {
                    sought.push((
                        time(first),
                        to_bytes(&actual[first * 2..(offset + frames) * 2]),
                    ));
                }
                offset += frames;
            }
            capture::worker_check_with_dolby_policy(
                Box::new(worker_source(c)),
                &full,
                std::time::Duration::from_millis(20),
                &sought,
                Some(3),
                fvid::audio_thread::DolbyPlaybackPolicy {
                    eac3_stereo_downmix: policy,
                    ..Default::default()
                },
            );
        }
    }
}
