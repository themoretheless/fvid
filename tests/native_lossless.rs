use fvid::{
    container::{mp4, webm},
    playback_native::NativeReader,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn directory(name: &str) -> Directory {
    let p = std::env::temp_dir().join(format!("fvid-lossless-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Directory(p)
}
#[test]
fn packets_timing_metadata_and_audio_survive_transcode() {
    for name in [
        "video.mp4",
        "hevc/main-ipb.mp4",
        "hevc/main10-ipb.mp4",
        "hevc/hdr10.mp4",
        "display/par-2x1.mp4",
        "audio/two-audio.mp4",
    ] {
        let source = fixture(name);
        let bytes = std::fs::read(&source).unwrap();
        let mut output = Cursor::new(Vec::new());
        let (stats, event) =
            fvid::native_lossless::write_mp4(&source, &mut output, None, None).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(!event.done);
        let mut input = mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let mut mkv =
            webm::WebmReader::open(Cursor::new(output.into_inner()), Default::default()).unwrap();
        mkv.scan_all().unwrap();
        assert_eq!(input.tracks().len(), mkv.tracks.len());
        assert_eq!(input.tags(), &mkv.tags);
        let mut raw = NativeReader::software(Cursor::new(&bytes), usize::MAX).unwrap();
        let mut times = Vec::new();
        while raw.read_frame_raw().unwrap().is_some() {
            let (a, b, scale) = raw.frame_interval().unwrap();
            times.push((
                (a * 1_000_000_000 / u128::from(scale)) as i64,
                ((b * 1_000_000_000 / u128::from(scale)) - (a * 1_000_000_000 / u128::from(scale)))
                    as u64,
            ));
        }
        for (i, track) in input.tracks().to_vec().iter().enumerate() {
            assert_eq!(track.name, mkv.tracks[i].name);
            let indices = mkv
                .packets
                .iter()
                .enumerate()
                .filter(|(_, p)| p.track == i as u64 + 1)
                .map(|(j, _)| j)
                .collect::<Vec<_>>();
            if track.handler == *b"vide" {
                assert_eq!(mkv.tracks[i].codec, "V_FFV1");
                assert_eq!(indices.len(), times.len());
                assert_eq!(stats.video_frames, times.len() as u64);
                for (j, &index) in indices.iter().enumerate() {
                    let p = &mkv.packets[index];
                    assert_eq!(p.pts_ns, times[j].0);
                    assert_eq!(p.duration_ns, Some(times[j].1));
                    assert!(!p.invisible);
                }
            } else {
                assert_eq!(mkv.tracks[i].codec, "A_AAC");
                for (j, index) in indices.iter().enumerate() {
                    let mut original = Vec::new();
                    input.read_packet(i, j, &mut original).unwrap();
                    assert_eq!(mkv.read_packet(*index).unwrap(), original);
                }
            }
        }
    }
}
#[test]
fn cli_api_cancel_and_atomic_publication() {
    let dir = directory("publish");
    let source = fixture("audio/two-audio.mp4");
    let destination = dir.0.join("result.mkv");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "transcode-lossless"])
        .arg(&source)
        .arg(&destination)
        .arg("--progress")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["backend"],
        "fvid"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .last()
            .unwrap()
            .contains("\"done\":true")
    );
    let before = std::fs::read(&destination).unwrap();
    assert!(fvid::native_export::transcode_mp4_ffv1(&source, &destination, None, None).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), before);
    let cancel = fvid::media_control::CancelFlag::default();
    let stop = cancel.clone();
    let hook = fvid::media_control::ProgressHook::new(move |e| {
        assert!(!e.done);
        if e.packets >= 2 {
            stop.cancel();
        }
    });
    let cancelled = dir.0.join("cancelled.mkv");
    assert!(
        fvid::native_export::transcode_mp4_ffv1(&source, &cancelled, Some(&cancel), Some(&hook))
            .is_err()
    );
    assert!(!cancelled.exists());
    assert!(
        !std::fs::read_dir(&dir.0).unwrap().any(|p| p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
    #[cfg(feature = "media")]
    {
        let path = dir.0.join("api.mkv");
        let stats = fvid::media::transcode_lossless(
            &source,
            &path,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn independent_decoder_preserves_transcoded_video_and_copied_audio() {
    use std::process::Command;
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let dir = directory("reference");
    for name in [
        "video.mp4",
        "hevc/main-ipb.mp4",
        "hevc/main10-ipb.mp4",
        "audio/two-audio.mp4",
    ] {
        let source = dir.0.join("source.mp4");
        if name == "audio/two-audio.mp4" {
            std::fs::copy(fixture(name), &source).unwrap();
        } else {
            let run = Command::new(&ffmpeg)
                .args(["-v", "error", "-y", "-i"])
                .arg(fixture(name))
                .arg("-i")
                .arg(fixture("audio/aac-native-edit.m4a"))
                .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"])
                .arg(&source)
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
        }
        let bytes = std::fs::read(&source).unwrap();
        let dest = dir.0.join(format!("result-{}.mkv", name.replace('/', "-")));
        fvid::native_export::transcode_mp4_ffv1(&source, &dest, None, None).unwrap();
        let decode = |path: &Path, map: &str, audio: bool| {
            let mut cmd = Command::new(&ffmpeg);
            cmd.args(["-v", "error", "-i"])
                .arg(path)
                .args(["-map", map]);
            if audio {
                cmd.args(["-f", "f32le", "-c:a", "pcm_f32le"]);
            } else {
                cmd.args([
                    "-fps_mode",
                    "passthrough",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    if name.contains("main10") {
                        "yuv420p10le"
                    } else {
                        "yuv420p"
                    },
                ]);
            }
            let run = cmd.arg("pipe:1").output().unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            run.stdout
        };
        assert!(
            decode(&source, "0:v:0", false) == decode(&dest, "0:v:0", false),
            "{name}: video"
        );
        let input = mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let mut audio_index = 0;
        for track in input.tracks().iter().filter(|t| t.handler == *b"soun") {
            let map = format!("0:a:{audio_index}");
            audio_index += 1;
            let before = decode(&source, &map, true);
            let after = decode(&dest, &map, true);
            let samples = if track.edits.is_empty() {
                u128::from(track.duration) * u128::from(track.sample_rate)
                    / u128::from(track.timescale)
            } else {
                (u128::from(track.edits[0].duration) * u128::from(track.sample_rate))
                    .div_ceil(u128::from(input.movie_timescale()))
            };
            let length = samples as usize * usize::from(track.channels) * 4;
            assert_eq!(after.len(), length, "{name} {map}: audible samples");
            assert!(before.len() >= length);
            assert!(after == before[..length], "{name} {map}: audio");
        }
    }
}

#[test]
fn rotation_aspect_hdr_and_completion_publication_are_preserved() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let dir = directory("display");
    for rotation in [90, 180, 270] {
        let mut bytes = std::fs::read(fixture("display/par-2x1.mp4")).unwrap();
        let matrix = bytes.windows(4).position(|w| w == b"tkhd").unwrap() + 44;
        let values: [i32; 4] = match rotation {
            90 => [0, 65536, -65536, 0],
            180 => [-65536, 0, 0, -65536],
            _ => [0, -65536, 65536, 0],
        };
        for (offset, value) in [0, 4, 12, 16].into_iter().zip(values) {
            bytes[matrix + offset..matrix + offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        let source = dir.0.join(format!("{rotation}.mp4"));
        std::fs::write(&source, &bytes).unwrap();
        let destination = dir.0.join(format!("{rotation}.mkv"));
        let published = destination.clone();
        let done = Arc::new(AtomicBool::new(false));
        let seen = done.clone();
        let hook = fvid::media_control::ProgressHook::new(move |event| {
            if event.done {
                assert!(published.exists());
                seen.store(true, Ordering::Relaxed);
            }
        });
        fvid::native_export::transcode_mp4_ffv1(&source, &destination, None, Some(&hook)).unwrap();
        assert!(done.load(Ordering::Relaxed));
        let input = mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
        let result = webm::WebmReader::open(
            Cursor::new(std::fs::read(destination).unwrap()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(result.tracks[0].rotation, rotation);
        let aspect = input.tracks()[0].pixel_aspect;
        assert_eq!(
            result.tracks[0].pixel_aspect(),
            if rotation == 180 {
                aspect
            } else {
                (aspect.1, aspect.0)
            }
        );
    }
    let source = fixture("hevc/hdr10.mp4");
    let mut result = Cursor::new(Vec::new());
    fvid::native_lossless::write_mp4(&source, &mut result, None, None).unwrap();
    let input = mp4::Mp4Reader::open(
        Cursor::new(std::fs::read(source).unwrap()),
        Default::default(),
    )
    .unwrap();
    let output =
        webm::WebmReader::open(Cursor::new(result.into_inner()), Default::default()).unwrap();
    assert_eq!(input.tracks()[0].hdr, output.tracks[0].hdr);
}

#[test]
fn edited_hevc_window_encodes_only_presented_pictures() {
    let dir = directory("edit");
    for name in ["hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"] {
        let mut bytes = std::fs::read(fixture(name)).unwrap();
        let input = mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let track = &input.tracks()[0];
        let mut times = (0..track.samples.len())
            .map(|i| track.samples.get(i).unwrap().pts)
            .collect::<Vec<_>>();
        times.sort();
        let begin = times[2];
        let end = times[times.len() - 2];
        let duration = u32::try_from(
            (end - begin) as u128 * u128::from(input.movie_timescale())
                / u128::from(track.timescale),
        )
        .unwrap();
        let at = bytes.windows(4).position(|w| w == b"elst").unwrap();
        assert_eq!(bytes[at + 4], 0);
        assert_eq!(&bytes[at + 8..at + 12], &1u32.to_be_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&duration.to_be_bytes());
        bytes[at + 16..at + 20].copy_from_slice(&i32::try_from(begin).unwrap().to_be_bytes());
        let source = dir.0.join(name.replace('/', "-"));
        std::fs::write(&source, &bytes).unwrap();
        let mut output = Cursor::new(Vec::new());
        fvid::native_lossless::write_mp4(&source, &mut output, None, None).unwrap();
        let mut raw = NativeReader::software(Cursor::new(bytes), usize::MAX).unwrap();
        let mut expected = Vec::new();
        while raw.read_frame_raw().unwrap().is_some() {
            let (a, b, s) = raw.frame_interval().unwrap();
            expected.push((
                (a * 1_000_000_000 / u128::from(s)) as i64,
                ((b * 1_000_000_000 / u128::from(s)) - (a * 1_000_000_000 / u128::from(s))) as u64,
            ));
        }
        let mut result =
            webm::WebmReader::open(Cursor::new(output.into_inner()), Default::default()).unwrap();
        result.scan_all().unwrap();
        assert_eq!(result.packets.len(), expected.len());
        for (p, (pts, duration)) in result.packets.iter().zip(expected) {
            assert_eq!(p.pts_ns, pts);
            assert_eq!(p.duration_ns, Some(duration));
            assert!(p.keyframe && !p.invisible);
        }
    }
}
