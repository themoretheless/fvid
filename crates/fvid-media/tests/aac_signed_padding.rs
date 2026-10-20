use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
}
fn cases() -> [(&'static str, usize, usize, usize, usize, usize); 5] {
    [
        ("head", 0, 17, 0, 0, 0),
        ("middle-head", 2, 17, 13, 0, 0),
        ("head-gap", 2, 17, 13, 0, 4768),
        ("offset-head", 0, 17, 13, 2400, 0),
        ("full-head", 0, 1024, 13, 0, 0),
    ]
}
fn oracle(head_packet: usize, head: usize, tail: usize, offset: usize, gap: usize) -> Vec<f32> {
    let raw = std::fs::read(root().join("aac-height-normal-ten.f32le")).unwrap();
    let input: Vec<f32> = raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let mut output = vec![0.; (6144 + offset + gap - tail) * 10];
    for i in 0..6 {
        let first = if i == head_packet { head } else { 0 };
        let last = if i == 5 { 1024 - tail } else { 1024 };
        let at = i * 1024 + offset + if i >= 2 { gap } else { 0 };
        output[(at + first) * 10..(at + last) * 10]
            .copy_from_slice(&input[(i * 1024 + first) * 10..(i * 1024 + last) * 10]);
    }
    output
}
fn same(actual: &[f32], expected: &[f32], label: &str) {
    assert_eq!(actual.len(), expected.len(), "{label}");
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 1e-6, "{label}: {a}/{b}");
    }
}
#[test]
fn signed_aac_padding_preserves_explicit_sample_windows_and_gaps() {
    for (name, index, head, tail, offset, gap) in cases() {
        let expected = oracle(index, head, tail, offset, gap);
        let bytes = std::fs::read(root().join(format!("aac-clock-{name}.mkv"))).unwrap();
        for interval in [
            None,
            Some((Duration::from_millis(50), Duration::from_millis(180))),
        ] {
            let begin = if interval.is_some() { 2400 } else { 0 };
            let end = if interval.is_some() {
                8640.min(expected.len() / 10)
            } else {
                expected.len() / 10
            };
            let mut out = Vec::new();
            let stats = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(&bytes),
                &mut out,
                interval,
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(stats.sample_frames, (end - begin) as u64, "{name}");
            let actual: Vec<f32> = out
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            same(&actual, &expected[begin * 10..end * 10], name);
        }
    }
}
#[test]
fn public_signed_aac_export_matches_the_same_independent_timeline() {
    let dir = std::env::temp_dir().join(format!("fvid-aac-signed-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for (name, index, head, tail, offset, gap) in cases() {
        let expected = oracle(index, head, tail, offset, gap);
        let source = root().join(format!("aac-clock-{name}.mkv"));
        let out = dir.join(format!("{name}.wav"));
        let stats = fvid_media::decode_audio(&source, &out, &Default::default()).unwrap();
        assert_eq!(stats.sample_frames, expected.len() as u64 / 10, "{name}");
        let mut file = std::fs::File::open(out).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut bytes = vec![0; info.data_bytes as usize];
        file.read_exact(&mut bytes).unwrap();
        let actual: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        same(&actual, &expected, name);
    }
}

#[test]
fn delayed_ps_internal_and_final_trims_preserve_the_declared_timeline() {
    for name in ["immediate", "late"] {
        let raw =
            std::fs::read(root().join(format!("aac-main-ps-adts-{name}-synthetic.aac"))).unwrap();
        let mut decoded = Vec::new();
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(raw),
            &mut decoded,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!((stats.channels, stats.sample_frames), (2, 24576));
        let mut expected: Vec<f32> = decoded
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        expected[(24576 - 2048 - 7) * 2..(24576 - 2048) * 2].fill(0.);
        expected.truncate((24576 - 13) * 2);
        let input = std::fs::read(root().join(format!("aac-clock-ps-{name}-tail.mkv"))).unwrap();
        for interval in [
            None,
            Some((Duration::from_millis(465), Duration::from_millis(470))),
        ] {
            let begin = if interval.is_some() { 22320 } else { 0 };
            let end = if interval.is_some() { 22560 } else { 24563 };
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(&input),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, (end - begin) as u64, "{name}");
            let actual: Vec<f32> = pcm
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            same(&actual, &expected[begin * 2..end * 2], name);
        }
    }
}

#[test]
fn a_full_tick_gap_at_an_exact_coded_boundary_is_not_quantization() {
    let raw = std::fs::read(root().join("aac-height-normal-ten.f32le")).unwrap();
    let mut expected: Vec<f32> = raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    expected.splice(3072 * 10..3072 * 10, std::iter::repeat_n(0., 48 * 10));
    let bytes = std::fs::read(root().join("aac-clock-tick-gap.mkv")).unwrap();
    let mut output = Vec::new();
    let stats = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
        Cursor::new(bytes),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 6192);
    let actual: Vec<f32> = output
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    same(&actual, &expected, "exact-tick gap");
}
