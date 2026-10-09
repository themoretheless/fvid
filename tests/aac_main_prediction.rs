use fvid_media::owned_aac::{NativeAacDecoder, aac_main_predictor::MainPredictor};
use serde_json::Value;
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-main-prediction.json"
    ))
    .unwrap()
}
fn config(v: &Value) -> Vec<u8> {
    v["case"]["asc"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn main_prediction_matches_independent_scalar_oracle_and_snapshot_replay() {
    let manifest = manifest();
    let mut bank = MainPredictor::new(64).unwrap();
    for (index, row) in manifest["oracle"].as_array().unwrap().iter().enumerate() {
        if row["short"].as_bool() == Some(true) {
            bank.short_window();
            continue;
        }
        let input: Vec<f32> = row["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap() as f32)
            .collect();
        let flags: Vec<bool> = row["used"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect();
        let reset = row["reset"].as_u64().map(|v| v as u8);
        let mut snapshot = bank.clone();
        let mut replay = input.clone();
        let mut output = input;
        bank.process(&mut output, &[0, 4, 34, 64], &flags, reset)
            .unwrap();
        snapshot
            .process(&mut replay, &[0, 4, 34, 64], &flags, reset)
            .unwrap();
        assert_eq!(output, replay);
        assert_eq!(bank, snapshot);
        for (line, (value, expected)) in output
            .iter()
            .zip(row["output_bits"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(
                u64::from(value.to_bits()),
                expected.as_u64().unwrap(),
                "frame {index} line {line}"
            );
        }
    }
}
#[test]
fn main_prediction_video_documents_current_profile_refusal() {
    let m = manifest();
    let asc = config(&m);
    for error in [
        NativeAacDecoder::new(&asc).err().unwrap().to_string(),
        fvid::codec::aac_native::NativeAacDecoder::new(&asc)
            .err()
            .unwrap()
            .to_string(),
    ] {
        assert_eq!(
            error,
            "only AAC-LC and AAC-SSR core configurations are implemented"
        );
    }
    let bytes = include_bytes!("fixtures/playback-errors/aac-main-prediction-synthetic.mp4");
    let blob = include_bytes!("fixtures/playback-errors/aac-main-prediction-packets.bin");
    let mut reader =
        fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(bytes), Default::default())
            .unwrap();
    let audio = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let mut lc_asc = asc.clone();
    lc_asc[0] = (lc_asc[0] & 7) | (2 << 3);
    let mut lc_control = NativeAacDecoder::new(&lc_asc).unwrap();
    for (i, row) in m["case"]["frames"].as_array().unwrap().iter().enumerate() {
        let mut packet = Vec::new();
        reader.read_packet(audio, i, &mut packet).unwrap();
        let start = row["offset"].as_u64().unwrap() as usize;
        let len = row["bytes"].as_u64().unwrap() as usize;
        assert_eq!(packet, &blob[start..start + len]);
        if i < 3 {
            let pcm = lc_control.decode(&packet).unwrap();
            assert_eq!(pcm.len(), 1024);
            assert!(pcm.iter().any(|v| *v != 0.0));
        } else if i == 3 {
            assert_eq!(
                lc_control.decode(&packet).unwrap_err().to_string(),
                "prediction is not allowed in AAC-LC"
            );
        }
    }
}
#[test]
#[ignore = "AAC Main predictor bank still needs channel/tool/checkpoint integration; refusal is not playback acceptance"]
fn main_prediction_video_has_native_playback_acceptance() {
    let m = manifest();
    let mut decoder = NativeAacDecoder::new(&config(&m)).unwrap();
    let blob = include_bytes!("fixtures/playback-errors/aac-main-prediction-packets.bin");
    for row in m["case"]["frames"].as_array().unwrap() {
        let start = row["offset"].as_u64().unwrap() as usize;
        let len = row["bytes"].as_u64().unwrap() as usize;
        let output = decoder.decode(&blob[start..start + len]).unwrap();
        assert_eq!(output.len(), 1024);
        assert!(output.iter().any(|v| *v != 0.0));
    }
}
