use fvid_media::owned_aac::{
    aac_native::NativeAacDecoder,
    aac_sbr_dsp::{Dsp, OutputRate},
    aac_sbr_history::Stream,
    bits::BitReader,
    config::AudioSpecificConfig,
};
use serde_json::Value;
const RAW: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-dependent-sbr-packets.bin");
const CORE: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-dependent-sbr-core.f32le");
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/he-aac-dependent-sbr-oracles.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn packet(row: &Value) -> &'static [u8] {
    let at = row["offset"].as_u64().unwrap() as usize;
    let len = row["bytes"].as_u64().unwrap() as usize;
    &RAW[at..at + len]
}
fn reference(c: &Value) -> Vec<f32> {
    let channels = c["channels"].as_u64().unwrap() as usize;
    let slots = c["slots"].as_u64().unwrap() as u8;
    let n = slots as usize * 64;
    let off = c["core_pcm"][0].as_u64().unwrap() as usize;
    let len = c["core_pcm"][1].as_u64().unwrap() as usize;
    let core: Vec<f32> = CORE[off..off + len * 4]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let mut dsp = Dsp::default();
    let mut syntax = Stream::default();
    let mut out = vec![];
    let mode = if c["bands"] == 32 {
        OutputRate::Core
    } else {
        OutputRate::Double
    };
    for (f, row) in c["frames"].as_array().unwrap().iter().enumerate() {
        let pcm = &core[f * n * channels..(f + 1) * n * channels];
        let planar: Vec<Vec<f32>> = (0..channels)
            .map(|ch| pcm.chunks_exact(channels).map(|r| r[ch]).collect())
            .collect();
        let refs: Vec<_> = planar.iter().map(Vec::as_slice).collect();
        let raw = hex(row["sbr"].as_str().unwrap());
        let rendered = if raw.is_empty() {
            dsp.process_upsampling(&refs, 48000, slots, mode).unwrap()
        } else {
            let mut bits = BitReader::new(&raw);
            let kind = bits.read(4).unwrap();
            let frame = syntax
                .read(&mut bits, raw.len() * 8, kind == 14, 48000, slots, channels)
                .unwrap();
            dsp.process(&frame, &refs, 48000, slots, mode).unwrap()
        };
        for i in 0..rendered[0].len() {
            for ch in 0..channels {
                out.push(rendered[ch][i] as f32)
            }
        }
    }
    out
}
#[test]
fn dependent_coupling_before_and_after_tns_reaches_sbr_with_independent_nonzero_core_pcm() {
    for c in manifest()["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AudioSpecificConfig::parse(&asc).unwrap();
        assert_eq!(config.program.as_ref().unwrap().coupling, vec![(false, 1)]);
        let mut d =
            NativeAacDecoder::new_with_output_rate(&asc, c["output_rate"].as_u64().unwrap() as u32)
                .unwrap();
        let mut actual = vec![];
        for row in c["frames"].as_array().unwrap() {
            let saved = d.checkpoint();
            let output = d.decode(packet(row)).unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(output, d.decode(packet(row)).unwrap());
            actual.extend(output);
        }
        let expected = reference(c);
        assert_eq!(actual.len(), expected.len());
        assert!(expected.iter().any(|v| v.abs() > 1e-5));
        for (n, (a, e)) in actual.iter().zip(&expected).enumerate() {
            assert!(
                (a - e).abs() < 2e-7,
                "{} {} {} sample {n}: {a} != {e}",
                c["slots"],
                c["point"],
                c["channels"]
            );
        }
        d.reset();
        let replay: Vec<f32> = c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| d.decode(packet(r)).unwrap())
            .collect();
        assert_eq!(actual, replay);
    }
}
