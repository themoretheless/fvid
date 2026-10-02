//! Explicit external Matroska metadata and rotation reference comparisons.
use fvid::{
    color::{
        hdr::{ColourDescription, HdrMetadata, MasteringDisplay},
        tonemap::ContentLight,
    },
    container::{
        matroska_write::{Encoding, PacketWriter, TrackSpec, VideoMetadata},
        mp4, webm,
    },
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
fn metadata() -> VideoMetadata {
    VideoMetadata {
        crop: [2, 2, 2, 2],
        pixel_aspect: (16, 15),
        colour: Some(ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        }),
        hdr: HdrMetadata {
            mastering: MasteringDisplay::from_corners(
                (0.708, 0.292),
                (0.170, 0.797),
                (0.131, 0.046),
                (0.3127, 0.3290),
                1000.0,
                0.005,
            ),
            light: ContentLight {
                max_cll: 1200.0,
                max_fall: 400.0,
            },
        },
    }
}
fn source() -> mp4::Track {
    let reader = mp4::Mp4Reader::open(
        std::fs::File::open(fixture("hevc/hdr10.mp4")).unwrap(),
        Default::default(),
    )
    .unwrap();
    reader
        .tracks()
        .iter()
        .find(|t| t.handler == *b"vide")
        .unwrap()
        .clone()
}
fn spec(track: &mp4::Track) -> TrackSpec<'_> {
    TrackSpec {
        encoding: Encoding::Hevc {
            configuration: &track.configuration,
            width: track.width.into(),
            height: track.height.into(),
        },
        name: "HDR picture",
        language: "und",
    }
}
fn write(meta: VideoMetadata) -> Vec<u8> {
    write_rotation(meta, 0)
}
fn write_rotation(meta: VideoMetadata, rotation: u16) -> Vec<u8> {
    let track = source();
    let mut input = mp4::Mp4Reader::open(
        std::fs::File::open(fixture("hevc/hdr10.mp4")).unwrap(),
        Default::default(),
    )
    .unwrap();
    let index = input
        .tracks()
        .iter()
        .position(|t| t.handler == *b"vide")
        .unwrap();
    let mut out = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new_with_options(
        &mut out,
        &[spec(&track)],
        &[fvid::container::matroska_write::TrackOptions {
            video: Some(meta),
            rotation,
            ..Default::default()
        }],
    )
    .unwrap();
    let mut packet = Vec::new();
    for i in 0..track.samples.len() {
        let sample = track.samples.get(i).unwrap();
        input.read_packet(index, i, &mut packet).unwrap();
        let pts = sample.pts as u64 * 1_000_000_000 / u64::from(track.timescale);
        let end = (sample.pts as u64 + u64::from(sample.duration)) * 1_000_000_000
            / u64::from(track.timescale);
        writer
            .write_packet(0, pts, end - pts, sample.sync, &packet)
            .unwrap();
    }
    writer.finish().unwrap();
    out.into_inner()
}

fn file_metadata_fixture() -> fvid::container::matroska_write::FileMetadata {
    use fvid::container::{FileTags, matroska_write::FileMetadata};
    FileMetadata {
        tags: FileTags {
            title: "Заголовок 🎬".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            genre: "Documentary".into(),
            date: "2026-09-29".into(),
            comment: "Line 1\nLine 2".into(),
            track: "3/12".into(),
            album_artist: "Album artist".into(),
            disc: "1/2".into(),
            publisher: "Publisher".into(),
            copyright: "© Author".into(),
            description: "Description".into(),
            rating: "4".into(),
        },
        chapters: vec![
            webm::Chapter {
                start_ns: 0,
                end_ns: Some(10_000_001),
                title: "Начало".into(),
            },
            webm::Chapter {
                start_ns: 10_000_001,
                end_ns: Some(20_000_002),
                title: "Середина".into(),
            },
            webm::Chapter {
                start_ns: 20_000_002,
                end_ns: None,
                title: "".into(),
            },
        ],
    }
}
fn write_file_metadata(meta: &fvid::container::matroska_write::FileMetadata) -> Vec<u8> {
    let data = std::fs::read(fixture("audio/aac-mono-44k.aac")).unwrap();
    let source = fvid::container::adts::Aac::parse(&data, &Default::default()).unwrap();
    let mut out = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new_with_metadata(
        &mut out,
        &[TrackSpec {
            encoding: Encoding::Aac {
                configuration: &source.frames[0].asc,
                sample_rate: source.sample_rate,
                channels: source.channels,
            },
            name: "Sound",
            language: "und",
        }],
        &[],
        meta,
    )
    .unwrap();
    writer
        .write_packet(0, 0, 30_000_003, true, source.packet(0))
        .unwrap();
    writer.finish().unwrap();
    out.into_inner()
}
fn independent_rotation_and_owned_y4m_snapshot_agree() {
    use std::{process::Command, time::Duration};
    let ffmpeg = std::env::var("FVID_REFERENCE_FFMPEG").unwrap();
    let directory = std::env::temp_dir().join(format!("fvid-mkv-rotation-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    let meta = VideoMetadata {
        crop: [0; 4],
        ..metadata()
    };
    let baseline = directory.join("baseline.mkv");
    std::fs::write(&baseline, write_rotation(meta, 0)).unwrap();
    let reference = |file: &Path, filter: &str| {
        let out = Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(file)
            .args([
                "-vf",
                filter,
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "yuv420p10le",
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
    for (angle, filter) in [
        (0, "null"),
        (90, "transpose=clock"),
        (180, "hflip,vflip"),
        (270, "transpose=cclock"),
    ] {
        let path = directory.join(format!("{angle}.mkv"));
        let data = write_rotation(meta, angle);
        std::fs::write(&path, &data).unwrap();
        assert!(
            reference(&path, "null") == reference(&baseline, filter),
            "independent orientation {angle}"
        );
        let mut reader =
            fvid::playback_native::NativeReader::software(Cursor::new(data), usize::MAX).unwrap();
        let raw = reader.read_frame_raw().unwrap().unwrap();
        let [w, h] = reader.dimensions();
        let expected = fvid::native_geometry::VideoGeometry::default()
            .apply_display(&raw, w, h, reader.rotation())
            .unwrap();
        let output = directory.join(format!("{angle}.y4m"));
        assert_eq!(
            fvid::native_export::export_y4m_interval(
                &path,
                &output,
                Some((Duration::ZERO, Duration::from_nanos(1)))
            )
            .unwrap(),
            1
        );
        let bytes = std::fs::read(output).unwrap();
        let start = bytes.windows(6).position(|v| v == b"FRAME\n").unwrap() + 6;
        assert!(
            bytes[start..] == expected.data,
            "export orientation {angle}"
        );
        let header =
            std::str::from_utf8(&bytes[..bytes.iter().position(|v| *v == b'\n').unwrap()]).unwrap();
        assert!(header.contains(&format!("W{w} H{h}")));
        let aspect = if angle == 90 || angle == 270 {
            "A15:16"
        } else {
            "A16:15"
        };
        assert!(header.contains(aspect), "{header}");
    }
    // Independent VP9 fixture exercises rotated 8-bit Planar8 export too.
    let vp9 = directory.join("vp9.mkv");
    let out = Command::new(&ffmpeg)
        .args(["-v", "error", "-display_rotation", "-90", "-i"])
        .arg(fixture("vp9/motion.webm"))
        .args(["-map", "0:v:0", "-c", "copy"])
        .arg(&vp9)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut reader = fvid::playback_native::NativeReader::software(
        std::io::BufReader::new(std::fs::File::open(&vp9).unwrap()),
        usize::MAX,
    )
    .unwrap();
    assert_eq!(reader.rotation(), 90);
    let raw = reader.read_frame_raw().unwrap().unwrap();
    assert!(matches!(&raw, fvid::playback_native::RawFrame::Planar8(_)));
    let [w, h] = reader.dimensions();
    assert_eq!([w, h], [96, 128]);
    let expected = fvid::native_geometry::VideoGeometry::default()
        .apply_display(&raw, w, h, 90)
        .unwrap();
    let output = directory.join("vp9.y4m");
    assert_eq!(
        fvid::native_export::export_y4m_interval(
            &vp9,
            &output,
            Some((Duration::ZERO, Duration::from_nanos(1)))
        )
        .unwrap(),
        1
    );
    let bytes = std::fs::read(output).unwrap();
    let start = bytes.windows(6).position(|v| v == b"FRAME\n").unwrap() + 6;
    assert!(bytes[start..] == expected.data);
}

fn independent_tools_confirm_written_hdr_and_real_mp4_colour_layout() {
    use std::process::Command;
    let ffprobe = std::env::var("FVID_REFERENCE_FFPROBE").unwrap();
    let ffmpeg = std::env::var("FVID_REFERENCE_FFMPEG").unwrap();
    let directory = std::env::temp_dir().join(format!("fvid-mkv-metadata-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    let mkv = directory.join("metadata.mkv");
    std::fs::write(
        &mkv,
        write(VideoMetadata {
            crop: [0; 4],
            ..metadata()
        }),
    )
    .unwrap();
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(&mkv)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let track = &json["streams"][0];
    assert_eq!(track["sample_aspect_ratio"], "16:15");
    assert_eq!(track["color_primaries"], "bt2020");
    assert_eq!(track["color_transfer"], "smpte2084");
    assert_eq!(track["color_space"], "bt2020nc");
    assert_eq!(track["color_range"], "tv");
    let side = track["side_data_list"].as_array().unwrap();
    let light = side
        .iter()
        .find(|v| v["side_data_type"] == "Content light level metadata")
        .unwrap();
    assert_eq!(light["max_content"], 1200);
    assert_eq!(light["max_average"], 400);
    assert!(
        side.iter()
            .any(|v| v["side_data_type"] == "Mastering display metadata")
    );
    let master = side
        .iter()
        .find(|v| v["side_data_type"] == "Mastering display metadata")
        .unwrap();
    for (key, expected) in [
        ("red_x", 0.708),
        ("red_y", 0.292),
        ("green_x", 0.170),
        ("green_y", 0.797),
        ("blue_x", 0.131),
        ("blue_y", 0.046),
        ("white_point_x", 0.3127),
        ("white_point_y", 0.3290),
        ("max_luminance", 1000.0),
        ("min_luminance", 0.005),
    ] {
        let (num, den) = master[key].as_str().unwrap().split_once('/').unwrap();
        let actual = num.parse::<f64>().unwrap() / den.parse::<f64>().unwrap();
        assert!(
            (actual - expected).abs() < 0.0001,
            "{key}: {actual} != {expected}"
        );
    }
    let mp4 = directory.join("colour.mp4");
    let out = Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(fixture("video.mp4"))
        .args([
            "-map",
            "0:v:0",
            "-c:v",
            "copy",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
            "-color_range",
            "pc",
            "-movflags",
            "write_colr",
        ])
        .arg(&mp4)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let reader =
        mp4::Mp4Reader::open(std::fs::File::open(mp4).unwrap(), Default::default()).unwrap();
    assert_eq!(
        reader.tracks()[0].colour,
        ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: true
        }
    );
}

fn independent_probe_reads_file_tags_and_chapters() {
    let d = std::env::temp_dir().join(format!("fvid-file-meta-{}.mka", std::process::id()));
    std::fs::write(&d, write_file_metadata(&file_metadata_fixture())).unwrap();
    let output = std::process::Command::new(std::env::var_os("FVID_REFERENCE_FFPROBE").unwrap())
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_chapters",
            "-of",
            "json",
        ])
        .arg(&d)
        .output()
        .unwrap();
    std::fs::remove_file(&d).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tags = value["format"]["tags"].as_object().unwrap();
    for (key, expected) in [
        ("TITLE", "Заголовок 🎬"),
        ("ARTIST", "Artist"),
        ("ALBUM", "Album"),
        ("GENRE", "Documentary"),
        ("DATE", "2026-09-29"),
        ("COMMENT", "Line 1\nLine 2"),
        ("track", "3/12"),
        ("ALBUM_ARTIST", "Album artist"),
        ("DISCNUMBER", "1/2"),
        ("PUBLISHER", "Publisher"),
        ("COPYRIGHT", "© Author"),
        ("DESCRIPTION", "Description"),
        ("RATING", "4"),
    ] {
        let got = tags
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .unwrap_or_else(|| panic!("missing {key}: {tags:?}"));
        assert_eq!(got.1.as_str(), Some(expected), "{key}");
    }
    let chapters = value["chapters"].as_array().unwrap();
    assert_eq!(chapters.len(), 3);
    assert_eq!(chapters[1]["start"], 10_000_001);
    assert_eq!(chapters[1]["end"], 20_000_002);
    assert_eq!(chapters[1]["tags"]["title"], "Середина");
}

fn main() {
    std::env::var("FVID_REFERENCE_FFMPEG").expect("set FVID_REFERENCE_FFMPEG");
    std::env::var("FVID_REFERENCE_FFPROBE").expect("set FVID_REFERENCE_FFPROBE");
    independent_rotation_and_owned_y4m_snapshot_agree();
    independent_tools_confirm_written_hdr_and_real_mp4_colour_layout();
    independent_probe_reads_file_tags_and_chapters();
    println!("Matroska rotation, HDR/MP4 colour and file tags/chapters reference suites passed");
}
