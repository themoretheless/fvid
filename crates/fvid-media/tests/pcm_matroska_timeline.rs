use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/pcm-timeline")
}
fn value(n: usize, c: usize) -> f64 {
    ((n * (2 * c + 1)) % 127) as f64 / 128. - 63. / 128.
}
#[test]
fn every_owned_pcm_width_preserves_container_origin_signed_windows_and_exact_intervals() {
    let base = root();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!("fvid-pcm-timeline-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    for c in manifest["cases"].as_array().unwrap() {
        if c["invalid"].as_bool().unwrap_or(false) {
            continue;
        }
        let name = c["file"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let offset = c["offset"].as_u64().unwrap() as usize;
        let gap = c["gap"].as_u64().unwrap() as usize;
        let delay = c["delay"].as_u64().unwrap() as i128;
        let total = c["samples"].as_u64().unwrap() as usize;
        let mut expected = vec![0f64; total * channels];
        for i in 0..6 {
            let at = (i * 64 + offset + if i >= 3 { gap } else { 0 }) as i128 - delay;
            let first = (-at).clamp(0, 64) as usize;
            let first = first.max(if i == c["head_packet"].as_u64().unwrap() as usize {
                c["head"].as_u64().unwrap() as usize
            } else {
                0
            });
            let last = if i == 5 {
                64 - c["tail"].as_u64().unwrap() as usize
            } else {
                64
            };
            for j in first..last {
                for ch in 0..channels {
                    expected[((at + j as i128) as usize) * channels + ch] = value(i * 64 + j, ch);
                }
            }
        }
        let bytes = std::fs::read(base.join(name)).unwrap();
        for interval in [
            None,
            Some((Duration::from_millis(1), Duration::from_millis(10))),
        ] {
            let begin = interval.map_or(0, |_| 48);
            let end = interval.map_or(total, |_| 480.min(total));
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
                Cursor::new(&bytes),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(stats.sample_frames, (end - begin) as u64, "{name}");
            let actual: Vec<_> = pcm
                .chunks_exact(4)
                .map(|b| f64::from(f32::from_le_bytes(b.try_into().unwrap())))
                .collect();
            assert_eq!(
                actual,
                &expected[begin * channels..end * channels],
                "{name}"
            );
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm_f64(
                Cursor::new(&bytes),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, (end - begin) as u64);
            let actual: Vec<_> = pcm
                .chunks_exact(8)
                .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
                .collect();
            assert_eq!(
                actual,
                &expected[begin * channels..end * channels],
                "{name}/f64"
            );
        }
        fvid_media::plan_decode_audio(&base.join(name), &Default::default(), &Default::default())
            .unwrap_or_else(|e| panic!("{name}/plan: {e}"));
        let output = dir.join(format!("{name}.wav"));
        let stats =
            fvid_media::decode_audio(&base.join(name), &output, &Default::default()).unwrap();
        assert_eq!(stats.sample_frames, total as u64);
        let mut f = std::fs::File::open(output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut f, None).unwrap();
        f.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut bytes = vec![0; info.data_bytes as usize];
        f.read_exact(&mut bytes).unwrap();
        let actual: Vec<f64> = if info.bits_per_sample == 64 {
            bytes
                .chunks_exact(8)
                .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
                .collect()
        } else {
            assert_eq!(info.bits_per_sample, 32);
            bytes
                .chunks_exact(4)
                .map(|b| f64::from(f32::from_le_bytes(b.try_into().unwrap())))
                .collect()
        };
        assert_eq!(actual, expected, "{name}/public");
    }
}

#[test]
fn malformed_pcm_cannot_publish_a_partial_wave() {
    let source = root().join("bad-last-f32-1.mkv");
    let out = std::env::temp_dir().join(format!("fvid-pcm-bad-stride-{}.wav", std::process::id()));
    assert!(!out.exists());
    let error = fvid_media::decode_audio(&source, &out, &Default::default()).unwrap_err();
    assert!(error.contains("incomplete channel frame"), "{error}");
    assert!(!out.exists());
}
