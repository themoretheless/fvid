use fvid::native_export::export_audio_pcm_selected as export;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir() -> Dir {
    let p = std::env::temp_dir().join(format!(
        "fvid-alac-media-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/alac")
        .join(name)
}
#[test]
fn cli_plan_intervals_gain_selection_and_api_use_owned_alac() {
    let d = dir();
    let source = fixture("stereo-24.m4a");
    assert!(fvid::native_media::is_alac_source(&source).unwrap());
    let full = d.0.join("full.f32le");
    let stats = export(&source, &full, None, 1.0, None, None, None, None, None).unwrap();
    let bytes = std::fs::read(&full).unwrap();
    assert_eq!(
        bytes.len() as u64,
        stats.sample_frames * u64::from(stats.channels) * 4
    );
    let dest = d.0.join("window.f32le");
    let from = Duration::from_millis(10);
    let to = Duration::from_millis(50);
    let window = export(
        &source,
        &dest,
        Some((from, to)),
        0.5,
        Some(1),
        None,
        Some(0),
        None,
        None,
    )
    .unwrap();
    let start = (from.as_nanos() * u128::from(stats.sample_rate)).div_ceil(1_000_000_000) as usize;
    let end = (to.as_nanos() * u128::from(stats.sample_rate)).div_ceil(1_000_000_000) as usize;
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|p| f32::from_le_bytes(p.try_into().unwrap()))
        .collect();
    let expected: Vec<u8> = (start..end)
        .flat_map(|i| ((samples[2 * i] + samples[2 * i + 1]) * 0.5 * 0.5).to_le_bytes())
        .collect();
    assert_eq!(window.sample_frames, (end - start) as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), expected);
    let cli = d.0.join("cli.f32le");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"])
        .arg(&source)
        .arg(&cli)
        .args([
            "--from",
            "0.01",
            "--to",
            "0.05",
            "--channels",
            "1",
            "--volume",
            "0.5",
            "--streams",
            "0",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read(&cli).unwrap(), expected);
    let plan = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "decode-audio"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(plan["streams"][0]["codec"], "alac");
    assert!(plan["graph"].is_null());
    {
        let output = d.0.join("api.f32le");
        let transform = fvid::media::AudioDecodeTransform {
            interval: Some((10000, 50000)),
            volume: Some(0.5),
            channels: Some(1),
            ..Default::default()
        };
        fvid::media::decode_audio_transformed(&source, &output, transform, &Default::default())
            .unwrap();
        assert_eq!(std::fs::read(output).unwrap(), expected);
    }
    let invalid = d.0.join("invalid.f32le");
    assert!(
        export(
            &source,
            &invalid,
            None,
            1.0,
            None,
            None,
            Some(1),
            None,
            None
        )
        .is_err()
    );
    assert!(!invalid.exists());
    assert!(fvid::native_export::export_aac_pcm(&source, &invalid).is_err());
    assert!(!invalid.exists());
    assert!(export(&source, &full, None, 1.0, None, None, None, None, None).is_err());
    assert_eq!(std::fs::read(full).unwrap(), bytes);
    let cancel = fvid::media_control::CancelFlag::default();
    cancel.cancel();
    assert!(
        export(
            &source,
            &invalid,
            None,
            1.0,
            None,
            None,
            None,
            Some(&cancel),
            None
        )
        .is_err()
    );
    assert!(!invalid.exists());
}
#[test]
fn owned_mix_and_merge_accept_alac_inputs() {
    let d = dir();
    let source = fixture("mono-16.m4a");
    let pcm = d.0.join("baseline.f32le");
    let original = export(&source, &pcm, None, 1.0, None, None, None, None, None).unwrap();
    let bytes = std::fs::read(&pcm).unwrap();
    let sources = vec![source.clone(), source];
    assert!(fvid::native_audio_mix::eligible(&sources).unwrap());
    let mixed = d.0.join("mix.wav");
    let stats = fvid::native_audio_mix::mix_audio(&sources, &mixed, &Default::default()).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.sample_frames, original.sample_frames);
    let decoded = d.0.join("mix.f32le");
    export(&mixed, &decoded, None, 1.0, None, None, None, None, None).unwrap();
    assert_eq!(std::fs::read(decoded).unwrap(), bytes);
    let merged = d.0.join("merge.wav");
    fvid::native_audio_mix::merge_audio(&sources, &merged).unwrap();
    let decoded = d.0.join("merge.f32le");
    export(&merged, &decoded, None, 1.0, None, None, None, None, None).unwrap();
    let expected: Vec<u8> = bytes
        .chunks_exact(4)
        .flat_map(|sample| sample.iter().chain(sample).copied())
        .collect();
    assert_eq!(std::fs::read(decoded).unwrap(), expected);
}
#[test]
fn damaged_packet_and_cookie_fail_without_publishing() {
    let d = dir();
    let bytes = std::fs::read(fixture("stereo-24.m4a")).unwrap();
    let reader =
        fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(&bytes), Default::default())
            .unwrap();
    let sample = &reader.tracks()[0].samples.get(1).unwrap();
    let mut corrupt = bytes.clone();
    let at = sample.offset as usize;
    corrupt[at..at + sample.size as usize].fill(0xff);
    let input = d.0.join("bad.m4a");
    std::fs::write(&input, corrupt).unwrap();
    assert!(fvid::native_media::is_owned_audio_source(&input).unwrap());
    {
        let output = d.0.join("api-bad.f32le");
        assert!(fvid::media::decode_audio(&input, &output, &Default::default()).is_err());
        assert!(!output.exists());
    }
    let output = d.0.join("bad.f32le");
    assert!(export(&input, &output, None, 1.0, None, None, None, None, None).is_err());
    assert!(!output.exists());
    let cookie = &reader.tracks()[0].configuration;
    let at = bytes
        .windows(cookie.len())
        .position(|p| p == cookie)
        .unwrap();
    let mut corrupt = bytes.clone();
    corrupt[at + 20..at + 24].copy_from_slice(&1u32.to_be_bytes());
    std::fs::write(&input, corrupt).unwrap();
    assert!(!fvid::native_media::is_owned_audio_source(&input).unwrap());
    assert!(export(&input, &output, None, 1.0, None, None, None, None, None).is_err());
    assert!(!output.exists());
    assert!(!std::fs::read_dir(&d.0).unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-")
    }));
}

#[test]
fn matroska_alac_uses_owned_timeline() {
    let d = dir();
    let source = fixture("stereo-24.mka");
    assert!(fvid::native_media::is_owned_audio_source(&source).unwrap());
    let info = fvid::native_media::audio_source_info_selected(&source, None).unwrap();
    assert_eq!(info.codec, "alac");
    let full = d.0.join("full.f32le");
    let stats = export(&source, &full, None, 1.0, None, None, None, None, None).unwrap();
    let bytes = std::fs::read(&full).unwrap();

    let window = d.0.join("window.f32le");
    let from = Duration::from_millis(10);
    let to = Duration::from_millis(50);
    export(
        &source,
        &window,
        Some((from, to)),
        1.0,
        None,
        None,
        Some(0),
        None,
        None,
    )
    .unwrap();
    let start = (from.as_nanos() * u128::from(stats.sample_rate)).div_ceil(1_000_000_000) as usize;
    let end = (to.as_nanos() * u128::from(stats.sample_rate)).div_ceil(1_000_000_000) as usize;
    let stride = usize::from(stats.channels) * 4;
    assert_eq!(
        std::fs::read(window).unwrap(),
        bytes[start * stride..end * stride]
    );
    {
        let output = d.0.join("api.f32le");
        fvid::media::decode_audio(&source, &output, &Default::default()).unwrap();
        assert_eq!(std::fs::read(output).unwrap(), bytes);
    }
}

#[test]
fn matroska_alac_delay_and_signed_padding_are_sample_exact() {
    fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
        let size = (body.len() as u32 | 0x1000_0000).to_be_bytes();
        [id, &size, body].concat()
    }
    let d = dir();
    let source = fixture("stereo-24.mka");
    let mut reader = fvid::container::webm::WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    let track = reader.tracks[0].clone();
    let rate = track.sample_rate;
    let channels = track.channels as usize;
    let first = reader.read_packet(0).unwrap();
    let second = reader.read_packet(1).unwrap();
    let mut decoder = fvid::codec::alac_decoder::AlacDecoder::new(
        &track.codec_private,
        rate as u32,
        channels as u16,
    )
    .unwrap();
    use fvid::audio::AudioDecode;
    let a = decoder.decode_encoded(&first, 0, 0).unwrap().unwrap().data;
    let b = decoder.decode_encoded(&second, 0, 0).unwrap().unwrap().data;
    let first_frames = a.len() / (channels * 4);
    let frames = first_frames + b.len() / (channels * 4);
    let whole = [a, b].concat();
    let header = element(
        &[0x1a, 0x45, 0xdf, 0xa3],
        &element(&[0x42, 0x82], b"matroska"),
    );
    let info = element(
        &[0x15, 0x49, 0xa9, 0x66],
        &element(&[0x2a, 0xd7, 0xb1], &1_000_000u32.to_be_bytes()),
    );
    let audio = element(
        &[0xe1],
        &[
            element(&[0xb5], &(rate as f64).to_be_bytes()),
            element(&[0x9f], &[channels as u8]),
        ]
        .concat(),
    );
    let delay = 100 * 1_000_000_000u64 / rate;
    let entry = element(
        &[0xae],
        &[
            element(&[0xd7], &[1]),
            element(&[0x83], &[2]),
            element(&[0x86], b"A_ALAC"),
            element(&[0x63, 0xa2], &track.codec_private),
            element(&[0x56, 0xaa], &delay.to_be_bytes()),
            audio,
        ]
        .concat(),
    );
    let tracks = element(&[0x16, 0x54, 0xae, 0x6b], &entry);
    for sign in [1i64, -1] {
        let padding = sign * (50 * 1_000_000_000u64 / rate) as i64;
        let timestamp = (first_frames as u64 * 1000 / rate) as i16;
        let block_header = [&[0x81][..], &timestamp.to_be_bytes(), &[0][..]].concat();
        let cluster = element(
            &[0x1f, 0x43, 0xb6, 0x75],
            &[
                element(&[0xe7], &[0]),
                element(&[0xa3], &[&[0x81, 0, 0, 0x80][..], &first].concat()),
                element(
                    &[0xa0],
                    &[
                        element(&[0xa1], &[block_header, second.clone()].concat()),
                        element(&[0x75, 0xa2], &padding.to_be_bytes()),
                    ]
                    .concat(),
                ),
            ]
            .concat(),
        );
        let input = d.0.join(format!("trim-{sign}.mka"));
        std::fs::write(
            &input,
            [
                header.clone(),
                element(
                    &[0x18, 0x53, 0x80, 0x67],
                    &[info.clone(), tracks.clone(), cluster].concat(),
                ),
            ]
            .concat(),
        )
        .unwrap();
        let output = d.0.join(format!("trim-{sign}.f32le"));
        let stats = export(&input, &output, None, 1.0, None, None, None, None, None).unwrap();
        let stride = channels * 4;
        let expected = if sign > 0 {
            whole[100 * stride..(frames - 50) * stride].to_vec()
        } else {
            [
                &whole[100 * stride..first_frames * stride],
                &whole[(first_frames + 50) * stride..],
            ]
            .concat()
        };
        assert_eq!(stats.sample_frames, (frames - 150) as u64);
        assert_eq!(std::fs::read(output).unwrap(), expected);
    }
}
