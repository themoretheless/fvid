#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};
fn check(name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-rice-boundaries");
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap();
    let frames = c["samples"].as_u64().unwrap() as usize;
    let depth = c["depth"].as_u64().unwrap();
    let counter = if name.starts_with("zero") {
        "zero_history_runs"
    } else if name.starts_with("small") {
        "masked_run_parameters"
    } else if name.starts_with("modifier") {
        "modifier_history_edges"
    } else {
        "max_runs"
    };
    assert!(
        c["stats"][counter].as_u64().unwrap() > 0,
        "fixture must exercise {counter}"
    );
    if name == "long-multiple-runs" {
        assert!(c["stats"]["zero_history_runs"].as_u64().unwrap() > 0);
    }

    let values = std::fs::read(root.join(format!("{name}.i32le"))).unwrap();
    let expected: Vec<f32> = values
        .chunks_exact(4)
        .map(|b| i32::from_le_bytes(b.try_into().unwrap()) as f32 / (1u64 << (depth - 1)) as f32)
        .collect();
    let cookie = std::fs::read(root.join(format!("{name}.cookie"))).unwrap();
    let packet = std::fs::read(root.join(format!("{name}.packet"))).unwrap();
    let mut decoder = fvid::codec::alac_decoder::AlacDecoder::new(&cookie, 48000, 1).unwrap();
    let pcm = decoder
        .decode_pcm(&packet)
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(pcm.len(), frames);
    assert_eq!(
        pcm.iter()
            .zip(&expected)
            .position(|(a, b)| a.to_bits() != b.to_bits()),
        None,
        "{name}: first wrong sample"
    );
    let expected: Vec<u8> = expected.into_iter().flat_map(f32::to_le_bytes).collect();
    for extension in ["mkv", "m4a"] {
        let source = root.join(format!("{name}.{extension}"));
        let bytes = std::fs::read(&source).unwrap();
        if extension == "mkv" {
            let mut video =
                fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                    .unwrap();
            for _ in 0..2 {
                assert!(video.read_frame_raw().unwrap().is_some());
            }
            assert!(video.read_frame_raw().unwrap().is_none());
        }
        for first in [0, 1, frames + 1] {
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
            reader.seek_to(if extension == "mkv" {
                (first as u64 * 1_000_000_000 / 48000) as i64
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
            let want = expected.repeat(2);
            assert_eq!(pcm.len(), want[first * 4..].len());
            assert_eq!(
                pcm.iter().zip(&want[first * 4..]).position(|(a, b)| a != b),
                None,
                "{name}/{extension}/{first}"
            );
        }
        let output = std::env::temp_dir().join(format!(
            "fvid-alac-rice-{}-{name}-{extension}.wav",
            std::process::id()
        ));
        struct Remove(std::path::PathBuf);
        impl Drop for Remove {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Remove(output.clone());
        fvid_media::decode_audio(&source, &output, &Default::default()).unwrap();
        let mut file = std::fs::File::open(&output).unwrap();
        let wave = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert!(wave.float);
        file.seek(SeekFrom::Start(wave.data_offset)).unwrap();
        let mut pcm = vec![0; wave.data_bytes as usize];
        file.read_exact(&mut pcm).unwrap();
        let want = expected.repeat(2);
        assert_eq!(pcm.len(), want.len());
        assert_eq!(
            pcm.iter().zip(&want).position(|(a, b)| a != b),
            None,
            "{name}/{extension} export"
        );
        let native_output = output.with_extension("f32le");
        let _native_cleanup = Remove(native_output.clone());
        fvid::native_export::export_audio_pcm_selected(
            &source,
            &native_output,
            None,
            1.0,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let pcm = std::fs::read(native_output).unwrap();
        assert_eq!(pcm.len(), want.len());
        assert_eq!(
            pcm.iter().zip(&want).position(|(a, b)| a != b),
            None,
            "{name}/{extension} native PCM export"
        );
    }
}
macro_rules! case {
    ($test:ident,$name:literal) => {
        #[test]
        fn $test() {
            check($name)
        }
    };
}
case!(zero_initial_history, "zero-initial-history");
case!(zero_history_after_run, "zero-history-after-run");
case!(small_limit_one, "small-limit-1");
case!(small_limit_two, "small-limit-2");
case!(small_limit_four, "small-limit-4");
case!(long_run_resumes_without_sign_modifier, "long-run-resume");
case!(terminal_run_65535, "terminal-run-65535");
case!(
    history_clamp_uses_coded_value_before_modifier,
    "modifier-history-edge"
);

case!(
    long_multiple_runs_reenter_zero_history,
    "long-multiple-runs"
);
