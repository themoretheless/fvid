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

#[test]
fn colour_hdr_crop_and_fractional_pixel_aspect_reach_the_native_player() {
    for (index, ratio) in [(1, 1), (16, 15), (64, 45), (u32::MAX, u32::MAX - 1)]
        .into_iter()
        .enumerate()
    {
        let mut expected = VideoMetadata {
            pixel_aspect: ratio,
            ..metadata()
        };
        expected.colour.as_mut().unwrap().full_range = index % 2 == 1;
        let data = write(expected);
        let reader = webm::WebmReader::open(Cursor::new(&data), Default::default()).unwrap();
        let track = &reader.tracks[0];
        assert_eq!(track.pixel_aspect(), ratio);
        assert_eq!(track.crop, expected.crop.map(u64::from));
        assert_eq!(track.colour, expected.colour.unwrap());
        assert_eq!(track.hdr, expected.hdr);
        let player =
            fvid::playback_native::NativeReader::software(Cursor::new(data), usize::MAX).unwrap();
        assert_eq!(player.pixel_aspect(), ratio);
        assert_eq!(player.insets(), expected.crop);
        assert_eq!(player.colour(), expected.colour.unwrap());
        assert_eq!(player.hdr(), expected.hdr);
    }
}

#[test]
fn malformed_metadata_fails_before_writing_output() {
    let source = source();
    let mut variants = Vec::new();
    for crop in [
        [u32::MAX, 0, 1, 0],
        [u32::from(source.width), 0, 0, 0],
        [0, 0, 0, u32::from(source.height)],
    ] {
        variants.push(VideoMetadata { crop, ..metadata() });
    }
    variants.push(VideoMetadata {
        pixel_aspect: (0, 1),
        ..metadata()
    });
    for value in [f32::NAN, f32::INFINITY, -1.0, 0.5, f32::MAX] {
        let mut m = metadata();
        m.hdr.light.max_cll = value;
        variants.push(m);
    }
    for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let mut m = metadata();
        m.hdr.mastering.as_mut().unwrap().red.x = value;
        variants.push(m);
    }
    let mut m = metadata();
    m.hdr.mastering.as_mut().unwrap().min_luminance = 2000.0;
    variants.push(m);
    for meta in variants {
        let mut out = Cursor::new(Vec::new());
        assert!(
            PacketWriter::new_with_video_metadata(&mut out, &[spec(&source)], &[Some(meta)])
                .is_err()
        );
        assert!(out.get_ref().is_empty());
    }
    let mut out = Cursor::new(Vec::new());
    assert!(
        PacketWriter::new_with_video_metadata(&mut out, &[spec(&source)], &[None, None]).is_err()
    );
    let audio = TrackSpec {
        encoding: Encoding::Aac {
            configuration: &[0x12, 0x10],
            sample_rate: 44100,
            channels: 2,
        },
        name: "",
        language: "",
    };
    assert!(
        PacketWriter::new_with_video_metadata(&mut out, &[audio], &[Some(metadata())]).is_err()
    );
    assert!(out.get_ref().is_empty());
}

#[test]
fn orientation_keeps_rgb_raw_seek_aspect_and_crop_consistent() {
    use fvid::{
        native_geometry::VideoGeometry,
        playback_native::{NativeReader, rotate_plane},
    };
    use std::time::Duration;
    let meta = VideoMetadata {
        crop: [1, 2, 3, 4],
        ..metadata()
    };
    let mut baseline =
        NativeReader::software(Cursor::new(write_rotation(meta, 0)), usize::MAX).unwrap();
    assert!(baseline.read_frame().unwrap());
    let [w, h] = baseline.dimensions();
    let rgb = baseline.rgb().to_vec();
    for angle in [0, 90, 180, 270] {
        let data = write_rotation(meta, angle);
        let mut reader = NativeReader::software(Cursor::new(&data), usize::MAX).unwrap();
        assert_eq!(reader.rotation(), angle);
        assert_eq!(
            reader.pixel_aspect(),
            if angle == 90 || angle == 270 {
                (15, 16)
            } else {
                (16, 15)
            }
        );
        assert_eq!(
            reader.insets(),
            match angle {
                90 => [4, 1, 2, 3],
                180 => [3, 4, 1, 2],
                270 => [2, 3, 4, 1],
                _ => [1, 2, 3, 4],
            }
        );
        assert!(reader.read_frame().unwrap());
        assert_eq!(
            reader.dimensions(),
            if angle == 90 || angle == 270 {
                [h, w]
            } else {
                [w, h]
            }
        );
        assert_eq!(reader.rgb(), rotate_plane(&rgb, w, h, angle, 3));
        let shown = reader.rgb().to_vec();
        reader.seek(Duration::ZERO).unwrap();
        assert_eq!(reader.rgb(), shown);
        let raw = reader.seek_raw(Duration::ZERO).unwrap().unwrap();
        let [dw, dh] = reader.dimensions();
        let normalized = VideoGeometry::default()
            .apply_display(&raw, dw, dh, reader.rotation())
            .unwrap();
        assert_eq!([normalized.width, normalized.height], [dw, dh]);
        let coded_rgb = raw.into_rgb(usize::MAX).unwrap();
        assert_eq!(rotate_plane(&coded_rgb, w, h, angle, 3), shown);
    }
    let mut output = Cursor::new(Vec::new());
    let source = source();
    let option = fvid::container::matroska_write::TrackOptions {
        rotation: 45,
        ..Default::default()
    };
    assert!(PacketWriter::new_with_options(&mut output, &[spec(&source)], &[option]).is_err());
    assert!(output.get_ref().is_empty());
}

#[test]
fn rotated_vp9_exports_oriented_planes_without_external_tools() {
    use fvid::{
        native_geometry::{Transpose, VideoGeometry},
        playback_native::NativeReader,
    };
    use std::time::Duration;
    let directory = std::env::temp_dir().join(format!("fvid-vp9-rotation-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    let mut source = NativeReader::software(
        Cursor::new(std::fs::read(fixture("vp9/motion.webm")).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let raw = source.read_frame_raw().unwrap().unwrap();
    let [w, h] = source.dimensions();
    let expected = VideoGeometry {
        transpose: Some(Transpose::Clock),
        ..Default::default()
    }
    .apply(&raw, w, h)
    .unwrap();
    let path = fixture("display/vp9-rot90.mkv");
    let stats = fvid::native_media::decode_video(&path).unwrap();
    assert_eq!(
        (stats.width, stats.height, stats.pixel_format.as_str()),
        (96, 128, "yuv420p")
    );
    let output = directory.join("frame.y4m");
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
    assert!(bytes[start..] == expected.data);
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
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

#[test]
#[ignore = "requires FVID_REFERENCE_FFPROBE and FVID_REFERENCE_FFMPEG"]
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
#[test]
fn file_tags_and_chapter_boundaries_survive_owned_writer() {
    let metadata = file_metadata_fixture();
    let bytes = write_file_metadata(&metadata);
    let mut reader = webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tags, metadata.tags);
    assert_eq!(reader.chapters, metadata.chapters);
    assert_eq!(reader.duration_ns, Some(30_000_003));
    for bad in [
        fvid::container::matroska_write::FileMetadata {
            tags: fvid::container::FileTags {
                title: "bad\0title".into(),
                ..Default::default()
            },
            ..Default::default()
        },
        fvid::container::matroska_write::FileMetadata {
            chapters: vec![webm::Chapter {
                start_ns: 2,
                end_ns: Some(1),
                title: "".into(),
            }],
            ..Default::default()
        },
    ] {
        let mut out = Cursor::new(Vec::new());
        let track = source();
        assert!(PacketWriter::new_with_metadata(&mut out, &[spec(&track)], &[], &bad).is_err());
        assert!(out.get_ref().is_empty());
    }
}
#[test]
#[ignore = "requires FVID_REFERENCE_FFPROBE"]
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

#[test]
fn library_ffv1_metadata_matches_frontend_bytes_and_refuses_invalid_tags() {
    use fvid_media::owned_matroska as owned;
    let root = metadata();
    let master = root.hdr.mastering.unwrap();
    let mut meta = owned::VideoMetadata {
        crop: root.crop,
        pixel_aspect: root.pixel_aspect,
        colour: Some(owned::ColourDescription {
            matrix: 9,
            transfer: 16,
            primaries: 9,
            full_range: false,
        }),
        hdr: owned::HdrMetadata {
            light: owned::ContentLight {
                max_cll: 1200.0,
                max_fall: 400.0,
            },
            mastering: Some(owned::MasteringDisplay {
                red: owned::Chromaticity {
                    x: master.red.x,
                    y: master.red.y,
                },
                green: owned::Chromaticity {
                    x: master.green.x,
                    y: master.green.y,
                },
                blue: owned::Chromaticity {
                    x: master.blue.x,
                    y: master.blue.y,
                },
                white: owned::Chromaticity {
                    x: master.white.x,
                    y: master.white.y,
                },
                max_luminance: 1000.0,
                min_luminance: 0.005,
            }),
        },
    };
    let samples = fvid::native_geometry::GeometryFrame {
        width: 8,
        height: 8,
        subsampling: Some([2, 2]),
        data: vec![0; 96],
    };
    let packet = fvid_media::owned_ffv1_encoder::encode(&samples, 8).unwrap();
    for angle in [0, 90, 180, 270] {
        let mut expected = Cursor::new(Vec::new());
        let mut writer = PacketWriter::new_with_options(
            &mut expected,
            &[TrackSpec {
                encoding: Encoding::Ffv1V1 {
                    width: 8,
                    height: 8,
                },
                name: "",
                language: "und",
            }],
            &[fvid::container::matroska_write::TrackOptions {
                video: Some(root),
                rotation: angle,
                default_duration_ns: 40_000_000,
                ..Default::default()
            }],
        )
        .unwrap();
        writer
            .write_packet(0, 0, 40_000_000, true, &packet)
            .unwrap();
        writer.finish().unwrap();
        let mut output = Cursor::new(Vec::new());
        let mut writer = owned::PacketWriter::new_ffv1_with_metadata(
            &mut output,
            8,
            8,
            Some(&meta),
            angle,
            40_000_000,
        )
        .unwrap();
        writer
            .write_packet(0, 0, 40_000_000, true, &packet)
            .unwrap();
        writer.finish().unwrap();
        assert_eq!(output.get_ref(), expected.get_ref());
        let reader =
            webm::WebmReader::open(Cursor::new(output.into_inner()), Default::default()).unwrap();
        let track = &reader.tracks[0];
        assert_eq!(track.crop, [2; 4]);
        assert_eq!(track.pixel_aspect(), (16, 15));
        assert_eq!(track.colour, root.colour.unwrap());
        assert_eq!(track.hdr, root.hdr);
    }
    for case in 0..5 {
        let mut invalid = meta;
        let angle = if case == 0 { 45 } else { 0 };
        match case {
            1 => invalid.crop = [8, 0, 0, 0],
            2 => invalid.pixel_aspect = (0, 1),
            3 => invalid.hdr.light.max_cll = 0.5,
            4 => invalid.hdr.mastering.as_mut().unwrap().red.x = f64::NAN,
            _ => {}
        }
        let mut output = Cursor::new(Vec::new());
        assert!(
            owned::PacketWriter::new_ffv1_with_metadata(
                &mut output,
                8,
                8,
                Some(&invalid),
                angle,
                0
            )
            .is_err()
        );
        assert!(output.get_ref().is_empty());
    }
    meta.colour.as_mut().unwrap().full_range = true;
    assert!(owned::video_element(8, 8, Some(&meta), 0).is_ok());
}

#[test]
fn library_file_tags_and_chapters_match_frontend_container_bytes() {
    use fvid_media::owned_matroska as owned;
    let original = file_metadata_fixture();
    let metadata = owned::FileMetadata {
        tags: original.tags.clone(),
        chapters: original.chapters.clone(),
    };
    let samples = fvid::native_geometry::GeometryFrame {
        width: 8,
        height: 8,
        subsampling: Some([2, 2]),
        data: vec![0; 96],
    };
    let packet = fvid_media::owned_ffv1_encoder::encode(&samples, 8).unwrap();
    let mut expected = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new_with_metadata(
        &mut expected,
        &[TrackSpec {
            encoding: Encoding::Ffv1V1 {
                width: 8,
                height: 8,
            },
            name: "",
            language: "und",
        }],
        &[],
        &original,
    )
    .unwrap();
    writer
        .write_packet(0, 0, 30_000_003, true, &packet)
        .unwrap();
    writer.finish().unwrap();
    let mut output = Cursor::new(Vec::new());
    let mut writer =
        owned::PacketWriter::new_ffv1_with_file_metadata(&mut output, 8, 8, None, 0, 0, &metadata)
            .unwrap();
    writer
        .write_packet(0, 0, 30_000_003, true, &packet)
        .unwrap();
    writer.finish().unwrap();
    assert_eq!(output.get_ref(), expected.get_ref());
    let mut reader =
        webm::WebmReader::open(Cursor::new(output.into_inner()), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tags, original.tags);
    assert_eq!(reader.chapters, original.chapters);
    for case in 0..3 {
        let mut bad = metadata.clone();
        match case {
            0 => bad.tags.title = "NUL\0title".into(),
            1 => bad.chapters[0].end_ns = Some(0),
            _ => bad.chapters[0].title = "NUL\0chapter".into(),
        }
        if case == 1 {
            bad.chapters[0].start_ns = 1;
        }
        let mut output = Cursor::new(Vec::new());
        assert!(
            owned::PacketWriter::new_ffv1_with_file_metadata(&mut output, 8, 8, None, 0, 0, &bad)
                .is_err()
        );
        assert!(output.get_ref().is_empty());
    }
    let mut tags = fvid_media::owned_file_tags::FileTags::default();
    assert!(tags.insert("tracknumber", "3/12"));
    assert!(tags.insert("PART_NUMBER", "4/12"));
    assert_eq!(tags.track, "3/12");
    assert!(tags.insert("albumartist", "Artist"));
    assert!(!tags.insert("unknown", "value"));
    assert!(!tags.insert("TITLE", ""));
}
