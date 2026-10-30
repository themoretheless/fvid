use fvid::media::CopyOptions;
use std::{io::Cursor, path::PathBuf};
fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/adts-packet-working-set/padded.aac")
}
#[test]
fn padded_fixture_decodes_nonzero_owned_pcm() {
    let mut pcm = Vec::new();
    fvid_media::owned_aac::decode_adts_pcm(
        Cursor::new(std::fs::read(source()).unwrap()),
        &mut pcm,
        None,
        &Default::default(),
    )
    .unwrap();
    let oracle = std::fs::read(source().with_file_name("oracle.f32le")).unwrap();
    assert_eq!(pcm.len(), oracle.len());
    let mut peak = 0f32;
    for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() <= 1e-6, "{a} != {b}");
        peak = peak.max(a.abs());
    }
    assert!(peak > 1e-5);
}
#[test]
fn native_adts_plans_charge_simultaneous_packet_and_index() {
    let small = CopyOptions {
        max_controlled_bytes: Some(17_000),
        ..Default::default()
    };
    let mut failures = Vec::new();
    let a = fvid::native_plan::remux_adts_with_options(&source(), &small);
    let b = fvid::native_plan::remux_with_options(&source(), &small);
    for (name, err) in [("explicit", a.err()), ("generic", b.err())] {
        if !err
            .as_deref()
            .is_some_and(|s| s.contains("controlled memory budget exceeded"))
        {
            failures.push(format!("{name}: {err:?}"));
        }
    }
    let two = [source(), source()];
    let small = CopyOptions {
        max_controlled_bytes: Some(33_200),
        ..Default::default()
    };
    let err = fvid::native_plan::concat_adts_with_options(&two, &small).err();
    if !err
        .as_deref()
        .is_some_and(|s| s.contains("controlled memory budget exceeded"))
    {
        failures.push(format!("concat: {err:?}"));
    }
    assert!(
        failures.is_empty(),
        "missing packet working-set charge: {failures:?}"
    );
}
#[test]
fn ample_budget_accepts_native_adts_plans() {
    let o = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    fvid::native_plan::remux_adts_with_options(&source(), &o).unwrap();
    fvid::native_plan::remux_with_options(&source(), &o)
        .unwrap()
        .unwrap();
    fvid::native_plan::concat_adts_with_options(&[source(), source()], &o).unwrap();
}

#[test]
fn native_mp4_publication_charges_packet_and_leaves_no_output_on_refusal() {
    let directory =
        std::env::temp_dir().join(format!("fvid-adts-working-set-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let destination = directory.join("copy.m4a");
    let small = CopyOptions {
        max_controlled_bytes: Some(17_000),
        ..Default::default()
    };
    let result = fvid::native_export::remux_adts_aac_with_options(&source(), &destination, &small);
    let error = result.err().map(|e| e.to_string());
    let exists = destination.exists();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("controlled memory budget exceeded")),
        "{error:?}"
    );
    assert!(!exists, "refused export published output");
}

#[test]
fn companion_movie_decodes_the_same_nonzero_audio() {
    let data = std::fs::read(source().with_file_name("padded.mp4")).unwrap();
    let mut pcm = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(data),
        &mut pcm,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        pcm,
        std::fs::read(source().with_file_name("oracle.f32le")).unwrap()
    );
}
#[test]
fn streaming_packets_keep_exact_bytes_limits_and_terminal_truncation() {
    let data = std::fs::read(source()).unwrap();
    let mut root = fvid::container::adts::StreamReader::open(Cursor::new(&data)).unwrap();
    let mut owned = fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&data)).unwrap();
    for i in 0..3 {
        let expected = &data[i * 1049 + 7..(i + 1) * 1049];
        assert_eq!(root.next_packet().unwrap().unwrap(), expected);
        assert_eq!(owned.next_packet().unwrap().unwrap(), expected);
    }
    for _ in 0..2 {
        assert!(root.next_packet().unwrap().is_none());
        assert!(owned.next_packet().unwrap().is_none());
    }
    assert!(
        fvid::container::adts::StreamReader::open_with_packet_limit(Cursor::new(&data), 1041)
            .is_err()
    );
    assert!(
        fvid_media::owned_aac::adts::StreamReader::open_with_packet_limit(Cursor::new(&data), 1041)
            .is_err()
    );
    let tail = &data[..data.len() - 1];
    let mut root = fvid::container::adts::StreamReader::open(Cursor::new(tail)).unwrap();
    let mut owned = fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(tail)).unwrap();
    for _ in 0..2 {
        assert!(root.next_packet().unwrap().is_some());
        assert!(owned.next_packet().unwrap().is_some());
    }
    assert!(root.next_packet().is_err());
    assert!(owned.next_packet().is_err());
    assert!(root.next_packet().unwrap().is_none());
    assert!(owned.next_packet().unwrap().is_none());
}
