#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/ac3-stereo-downmix")
}
fn decode(name: &str, rate: u32, ch: u16) -> Vec<f32> {
    let raw = std::fs::read(root().join(name)).unwrap();
    let mut d = fvid::codec::ac3_decoder::Ac3Decoder::new(&[], rate, ch)
        .unwrap()
        .with_stereo_downmix(fvid::codec::ac3_decoder::StereoDownmixPolicy {
            mode: fvid::codec::ac3_decoder::StereoDownmixMode::LtRt,
            headroom: false,
        });
    d.decode_complete(&raw, 0, 4608)
        .unwrap()
        .unwrap()
        .data
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
#[test]
fn requested_ltrt_uses_opposite_surround_polarity() {
    let native = decode("51-control.ac3", 48000, 6);
    let actual = decode("51-b6-c0-s0.ac3", 48000, 2);
    let mut error = 0f32;
    for i in 0..1280 {
        let n = &native[i * 6..i * 6 + 6];
        let l = (n[0] + 1.414 * n[2] - 0.841 * n[4] - 0.841 * n[5]).clamp(-1., 1.);
        let r = (n[1] + 1.414 * n[2] + 0.841 * n[4] + 0.841 * n[5]).clamp(-1., 1.);
        error = error
            .max((actual[i * 2] - l).abs())
            .max((actual[i * 2 + 1] - r).abs());
    }
    assert!(error < 3e-7, "Lt/Rt scalar-matrix error {error}");
}
use fvid::codec::ac3_decoder::{StereoDownmixMode as Mode, StereoDownmixPolicy as Policy};
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn to_samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn decode_policy(name: &str, rate: u32, ch: u16, policy: Policy) -> Vec<f32> {
    let raw = std::fs::read(root().join(name)).unwrap();
    let mut d = fvid::codec::ac3_decoder::Ac3Decoder::new(&[], rate, ch)
        .unwrap()
        .with_stereo_downmix(policy);
    to_samples(&d.decode_complete(&raw, 0, 4608).unwrap().unwrap().data)
}
fn oracle(c: &serde_json::Value, native: &[f32], policies: [Policy; 3]) -> Vec<f32> {
    let centre = [1.414f32, 1.189, 1., 0.841, 0.707, 0.595, 0.5, 0.];
    let surround = [0.841f32, 0.841, 0.841, 0.841, 0.707, 0.595, 0.5, 0.];
    let layout = c["layout"].as_u64().unwrap();
    let channels = c["channels"].as_u64().unwrap() as usize;
    let has_c = matches!(layout, 3 | 5 | 7);
    let rear: &[usize] = match layout {
        3 => &[],
        4 => &[2],
        5 => &[3],
        6 => &[2, 3],
        7 => &[4, 5],
        _ => panic!("unexpected geometry"),
    };
    let mut out = Vec::new();
    let mut offset = 0;
    for (block, frames) in [1280, 1536, 1536].into_iter().enumerate() {
        let policy = policies[block];
        let mix = &c["mixes"][block];
        let stated = c["bsid"] == 6 && !mix.is_null();
        let ltrt = match policy.mode {
            Mode::LoRo => false,
            Mode::LtRt => true,
            Mode::Automatic => stated && mix[0] == 1,
        };
        let (cg, sg) = if stated {
            (
                centre[mix[if ltrt { 1 } else { 3 }].as_u64().unwrap() as usize],
                surround[mix[if ltrt { 2 } else { 4 }].as_u64().unwrap() as usize],
            )
        } else if ltrt {
            (0.707, 0.707)
        } else {
            (
                c["basic_centre"].as_f64().unwrap() as f32,
                c["basic_surround"].as_f64().unwrap() as f32,
            )
        };
        let rear_weight = if rear.len() == 1 && !ltrt {
            0.7 * sg
        } else {
            sg
        };
        let sum = 1.
            + if has_c { cg } else { 0. }
            + if ltrt {
                sg * rear.len() as f32
            } else if rear.is_empty() {
                0.
            } else {
                rear_weight
            };
        let scale = if policy.headroom {
            1. / sum.max(1.)
        } else {
            1.
        };
        for i in offset..offset + frames {
            let n = &native[i * channels..(i + 1) * channels];
            let common = if has_c { cg * n[2] } else { 0. };
            let (ls, rs) = if ltrt {
                let s = rear.iter().map(|&j| sg * n[j]).sum::<f32>();
                (-s, s)
            } else {
                match rear {
                    [] => (0., 0.),
                    [j] => (rear_weight * n[*j], rear_weight * n[*j]),
                    [l, r] => (sg * n[*l], sg * n[*r]),
                    _ => unreachable!(),
                }
            };
            out.extend([
                ((n[0] + common + ls) * scale).clamp(-1., 1.),
                ((n[1] + common + rs) * scale).clamp(-1., 1.),
            ]);
        }
        offset += frames;
    }
    out
}
fn close(actual: &[f32], expected: &[f32], label: &str) {
    assert_eq!(actual.len(), expected.len(), "{label}");
    let error = actual
        .iter()
        .zip(expected)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(error < 4e-7, "{label}: scalar-matrix error {error}");
}
#[test]
fn every_stereo_mode_and_headroom_matches_independent_matrix_in_all_authored_layouts() {
    for c in manifest()["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        let ch = c["channels"].as_u64().unwrap() as u16;
        let native = decode_policy(c["control"].as_str().unwrap(), rate, ch, Policy::default());
        assert_eq!(native.len(), 4352 * ch as usize);
        for channel in 0..ch as usize {
            assert!(
                native
                    .chunks_exact(ch as usize)
                    .any(|n| n[channel].abs() > 1e-5),
                "{name}: silent native plane {channel}"
            );
        }
        for mode in [Mode::LoRo, Mode::LtRt, Mode::Automatic] {
            for headroom in [false, true] {
                let policy = Policy { mode, headroom };
                let expected = oracle(c, &native, [policy; 3]);
                let actual = decode_policy(&format!("{name}.ac3"), rate, 2, policy);
                close(&actual, &expected, &format!("{name} {policy:?}"));
            }
        }
    }
}
#[test]
fn trait_policy_changes_apply_at_frame_boundary_and_survive_reset() {
    for name in ["51-b6-c0-s0", "layout5-b6-c3-s4", "51-b8-c0-s0"] {
        let m = manifest();
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        let ch = c["channels"].as_u64().unwrap() as u16;
        let native = decode_policy(c["control"].as_str().unwrap(), rate, ch, Policy::default());
        let raw = std::fs::read(root().join(format!("{name}.ac3"))).unwrap();
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
        let mut d = fvid::codec::make_audio_decoder("A_AC3", &[], rate, 2, 0).unwrap();
        let mut actual = Vec::new();
        for (i, p) in raw
            .chunks_exact(c["frame_bytes"].as_u64().unwrap() as usize)
            .enumerate()
        {
            d.set_ac3_stereo_downmix(policies[i]);
            actual.extend(to_samples(
                &d.decode_complete(p, (i * 1536) as u64, 1536)
                    .unwrap()
                    .unwrap()
                    .data,
            ));
        }
        close(&actual, &oracle(c, &native, policies), name);
        d.reset();
        let repeat = to_samples(&d.decode_complete(&raw, 0, 4608).unwrap().unwrap().data);
        close(
            &repeat,
            &oracle(c, &native, [policies[2]; 3]),
            "reset retained stereo policy",
        );
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
        "A_AC3"
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
            4352 * 1_000_000_000 / self.rate as u64,
        ))
    }
    fn make_decoder(&self) -> fvid::Result<Box<dyn fvid::audio::AudioDecode>> {
        fvid::codec::make_audio_decoder("A_AC3", &[], self.rate, 2, 0)
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
            pts: (self.index * 1536) as i64,
            duration: 1536,
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
        let at = (at.max(0) as u64).min(4352);
        self.floor.set(at);
        self.index = if at == 4352 { 3 } else { 0 };
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
    let raw = std::fs::read(root().join(format!("{}.ac3", c["name"].as_str().unwrap()))).unwrap();
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
fn explicit_stereo_output_worker_preserves_policy_pcm_seek_and_rewind() {
    for name in ["51-b6-c0-s0", "layout5-b6-c3-s4"] {
        let m = manifest();
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        let ch = c["channels"].as_u64().unwrap() as u16;
        let native = decode_policy(c["control"].as_str().unwrap(), rate, ch, Policy::default());
        for mode in [Mode::LoRo, Mode::LtRt, Mode::Automatic] {
            let policy = Policy {
                mode,
                headroom: true,
            };
            let expected = oracle(c, &native, [policy; 3]);
            // Use verified decoder bytes for exact worker comparison, after the
            // independently computed scalar PCM has accepted the matrix.
            let actual = decode_policy(&format!("{name}.ac3"), rate, 2, policy);
            close(&actual, &expected, name);
            let to_bytes = |x: &[f32]| x.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
            let mut offset = 0;
            let mut full = Vec::new();
            let mut sought = Vec::new();
            let seek_sample = (u64::from(rate) * 55 / 1000) as usize;
            for frames in [1280, 1536, 1536] {
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
                std::time::Duration::from_millis(55),
                &sought,
                Some(3),
                fvid::audio_thread::DolbyPlaybackPolicy {
                    ac3_stereo_downmix: policy,
                    ..Default::default()
                },
            );
        }
    }
}
