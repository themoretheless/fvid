//! Explicit lossless MP4 export, copied audio and spatial transform comparisons.
use fvid::{
    container::{mp4, webm},
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
fn spatial_request() -> fvid::media_info::LosslessTransform {
    use fvid::media_info::{CropRect, LosslessTransform, PadRect, ScaleSize, TransposeMode};
    LosslessTransform {
        crop: Some(CropRect {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        }),
        horizontal_flip: true,
        vertical_flip: true,
        transpose: Some(TransposeMode::Clock),
        pad: Some(PadRect {
            width: 16,
            height: 12,
            x: 2,
            y: 2,
        }),
        scale: Some(ScaleSize {
            width: 12,
            height: 8,
        }),
        chromashift: Some("cbh=1:cbv=-2:crh=-3:crv=2:edge=wrap".into()),
        negate: Some("1".into()),
        sobel: Some("planes=1:scale=0.125".into()),
        dilation: Some("coordinates=170:threshold0=3:threshold1=0:threshold2=0".into()),
        ..Default::default()
    }
}
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

fn independent_pixels_match_spatial_transforms_and_rotation() {
    use std::process::Command;
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let dir = directory("spatial-reference");
    let mut failures = Vec::new();
    for (index, name) in [
        "video.mp4",
        "hevc/main-ipb.mp4",
        "hevc/main10-ipb.mp4",
        "display/par-2x1.mp4",
    ]
    .iter()
    .enumerate()
    {
        let mut bytes = std::fs::read(fixture(name)).unwrap();
        if index == 3 {
            let matrix = bytes.windows(4).position(|w| w == b"tkhd").unwrap() + 44;
            for (offset, value) in [0, 4, 12, 16].into_iter().zip([0i32, 65536, -65536, 0]) {
                bytes[matrix + offset..matrix + offset + 4].copy_from_slice(&value.to_be_bytes());
            }
        }
        let source = dir.0.join(format!("source{index}.mp4"));
        std::fs::write(&source, bytes).unwrap();
        for mode in 0..9 {
            let request = match mode {
                0 => spatial_request(),
                8 => {
                    let mut request = spatial_request();
                    request.pixelize = Some("3:5:max:7".into());
                    request
                }
                1 => fvid::media_info::LosslessTransform {
                    horizontal_flip: true,
                    ..Default::default()
                },
                2 => fvid::media_info::LosslessTransform {
                    erosion: Some("coordinates=1".into()),
                    ..Default::default()
                },
                _ => {
                    let mut r = spatial_request();
                    r.chromashift = None;
                    r.negate = None;
                    r.sobel = None;
                    r.dilation = None;
                    if mode >= 4 {
                        r.scale = None;
                    }
                    if mode >= 5 {
                        r.pad = None;
                    }
                    if mode >= 6 {
                        r.transpose = None;
                    }
                    if mode >= 7 {
                        r.horizontal_flip = false;
                        r.vertical_flip = false;
                    }
                    r
                }
            };
            let filter = match mode {
                8 => {
                    "crop=8:8:0:0,hflip,vflip,transpose=clock,pad=16:12:2:2,scale=12:8:flags=neighbor,negate,sobel=planes=1:scale=0.125,pixelize=3:5:max:7,dilation=coordinates=170:threshold0=3:threshold1=0:threshold2=0,chromashift=cbh=1:cbv=-2:crh=-3:crv=2:edge=wrap"
                }
                0 => {
                    "crop=8:8:0:0,hflip,vflip,transpose=clock,pad=16:12:2:2,scale=12:8:flags=neighbor,negate,sobel=planes=1:scale=0.125,dilation=coordinates=170:threshold0=3:threshold1=0:threshold2=0,chromashift=cbh=1:cbv=-2:crh=-3:crv=2:edge=wrap"
                }
                1 => "hflip",
                2 => "erosion=coordinates=1",
                3 => {
                    "crop=8:8:0:0,hflip,vflip,transpose=clock,pad=16:12:2:2,scale=12:8:flags=neighbor"
                }
                4 => "crop=8:8:0:0,hflip,vflip,transpose=clock,pad=16:12:2:2",
                5 => "crop=8:8:0:0,hflip,vflip,transpose=clock",
                6 => "crop=8:8:0:0,hflip,vflip",
                _ => "crop=8:8:0:0",
            };
            let (geometry, filters) = fvid::native_lossless::configuration(&request).unwrap();
            let dest = dir.0.join(format!("output{index}-{mode}.mkv"));
            fvid::native_export::transcode_mp4_ffv1_transformed(
                &source, &dest, &geometry, &filters, None, None,
            )
            .unwrap();
            let decode = |path: &Path, vf: Option<&str>| {
                let mut command = Command::new(&ffmpeg);
                command
                    .args(["-v", "error", "-i"])
                    .arg(path)
                    .args(["-map", "0:v:0"]);
                if let Some(vf) = vf {
                    command.args(["-vf", vf]);
                }
                let out = command
                    .args([
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
                        "pipe:1",
                    ])
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                out.stdout
            };
            let expected = decode(&source, Some(filter));
            let actual = decode(&dest, None);
            assert_eq!(expected.len(), actual.len(), "{name} mode {mode}");
            if let Some(i) = expected.iter().zip(&actual).position(|(a, b)| a != b) {
                failures.push(format!(
                    "{name} mode {mode} byte {i}: ref={} own={}",
                    expected[i], actual[i]
                ));
            }
            let reader = webm::WebmReader::open(
                Cursor::new(std::fs::read(&dest).unwrap()),
                Default::default(),
            )
            .unwrap();
            assert_eq!(reader.tracks[0].rotation, 0);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("Set FVID_REFERENCE_FFMPEG for this explicit reference benchmark");
    independent_decoder_preserves_transcoded_video_and_copied_audio();
    independent_pixels_match_spatial_transforms_and_rotation();
    println!("Lossless MP4 export and spatial reference comparisons passed");
}
