use fvid::{
    native_geometry::{GeometryFrame, VideoGeometry},
    native_shuffleplanes::{ShufflePlanes, apply},
    playback_native::NativeReader,
};
use std::{io::Cursor, path::Path};
const CASES: [(&str, &[u8], &[u8], u8, &str); 4] = [
    (
        "shuffleplanes-444-8",
        include_bytes!("fixtures/playback-errors/shuffleplanes-444-8.mkv"),
        include_bytes!("fixtures/playback-errors/shuffleplanes-444-8.yuv"),
        8,
        "1:2:0:0",
    ),
    (
        "shuffleplanes-420-10",
        include_bytes!("fixtures/playback-errors/shuffleplanes-420-10.mkv"),
        include_bytes!("fixtures/playback-errors/shuffleplanes-420-10.yuv"),
        10,
        "0:2:1:0",
    ),
    (
        "shuffleplanes-420-10-promote",
        include_bytes!("fixtures/playback-errors/shuffleplanes-420-10-promote.mkv"),
        include_bytes!("fixtures/playback-errors/shuffleplanes-420-10-promote.yuv"),
        10,
        "1:2:0:0",
    ),
    (
        "shuffleplanes-444-16",
        include_bytes!("fixtures/playback-errors/shuffleplanes-444-16.mkv"),
        include_bytes!("fixtures/playback-errors/shuffleplanes-444-16.yuv"),
        16,
        "2:2:0:0",
    ),
];
#[test]
fn owned_plane_shuffle_matches_analytical_oracles_at_source_precision() {
    for (_, input, expected, depth, args) in CASES {
        let mut reader = NativeReader::software(Cursor::new(input), usize::MAX).unwrap();
        let mut output = Vec::new();
        while let Some(raw) = reader.read_frame_raw().unwrap() {
            let [w, h] = reader.dimensions();
            let mut frame = VideoGeometry::default().apply(&raw, w, h).unwrap();
            apply(ShufflePlanes::parse(args).unwrap(), &mut frame, depth).unwrap();
            output.extend(frame.data);
        }
        assert_eq!(output, expected);
    }
    let mut rgb = GeometryFrame {
        width: 8,
        height: 16,
        subsampling: None,
        data: include_bytes!("fixtures/playback-errors/shuffleplanes-rgb.rgb").to_vec(),
    };
    apply(ShufflePlanes::parse("1:2:0:0").unwrap(), &mut rgb, 8).unwrap();
    assert_eq!(
        rgb.data,
        include_bytes!("fixtures/playback-errors/shuffleplanes-rgb-reference.rgb")
    );
}
#[test]
fn shared_decode_and_cli_execute_shuffle_without_legacy_backend() {
    for (name, _, _, _, args) in CASES {
        for extension in ["mkv","y4m"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/playback-errors/{name}.{extension}"));
        if extension == "y4m" {
            let info=fvid::native_probe::try_probe_as(&path,None).unwrap().expect("Y4M probe must remain owned");
            assert_eq!(info.duration_us,Some(80000));assert_eq!(info.streams[0].duration,Some(2));
        }
        let request = fvid::media_info::DecodeTransform {
            shuffleplanes: Some(args.into()),
            ..Default::default()
        };
        let stats = fvid::native_media::decode_video_request(&path, &request).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.video_frames, 2);
        let command = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "decode"])
            .arg(&path)
            .args(["--shuffleplanes", args])
            .output()
            .unwrap();
        assert!(
            command.status.success(),
            "{}",
            String::from_utf8_lossy(&command.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&command.stdout).unwrap()["backend"],
            "fvid"
        );
        assert_eq!(
            fvid::media::decode_video_transformed(&path, request)
                .unwrap()
                .backend,
            "fvid"
        );
        }
    }
}
#[test]
fn lossless_export_preserves_shuffled_plane_bytes_and_plans_owned_filter() {
    let dir = std::env::temp_dir().join(format!("fvid-shuffleplanes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for (name, _, expected, _, args) in CASES {
        for extension in ["mkv","y4m"] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/playback-errors/{name}.{extension}"));
        let request = fvid::media_info::LosslessTransform {
            shuffleplanes: Some(args.into()),
            ..Default::default()
        };
        assert!(fvid::native_lossless::supports(&request));
        let plan = fvid::native_plan::transcode_lossless(&source, &request).unwrap();
        assert!(
            plan.steps
                .iter()
                .any(|step| step.detail.contains("FVid shuffleplanes="))
        );
        let (geometry, filters) = fvid::native_lossless::configuration(&request).unwrap();
        let destination = dir.join(format!("{name}-{extension}.mkv"));
        fvid::native_export::transcode_ffv1_transformed(
            &source,
            &destination,
            &geometry,
            &filters,
            None,
            None,
        )
        .unwrap();
        let mut reader = NativeReader::software(
            std::io::BufReader::new(std::fs::File::open(destination).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let mut actual = Vec::new();
        while let Some(raw) = reader.read_frame_raw().unwrap() {
            let [w, h] = reader.dimensions();
            actual.extend(VideoGeometry::default().apply(&raw, w, h).unwrap().data);
        }
        assert_eq!(actual, expected);
        }
    }
}
#[test]
fn absent_planes_and_invalid_storage_are_explicit_atomic_refusals() {
    for args in ["3:1:2", "4:1:2", "map4=0", "0:1:2:3:0", "map0=no"] {
        assert!(ShufflePlanes::parse(args).is_err(), "{args}");
    }
    let mut frame = GeometryFrame {
        width: 2,
        height: 2,
        subsampling: Some([2, 2]),
        data: vec![1, 2, 3, 4, 5, 6],
    };
    apply(ShufflePlanes::parse("1:2:0:0").unwrap(), &mut frame, 8).unwrap();
    assert_eq!(frame.subsampling, Some([1, 1]));
    assert_eq!(frame.data, vec![5, 5, 5, 5, 6, 6, 6, 6, 1, 2, 3, 4]);
    frame.data.pop();
    let original = frame.data.clone();
    assert!(apply(ShufflePlanes::parse("0:2:1:0").unwrap(), &mut frame, 8).is_err());
    assert_eq!(frame.data, original);
}

#[test]
fn high_depth_y4m_accepts_exact_samples_and_rewind() {
    for (data,depth) in [(include_bytes!("fixtures/playback-errors/shuffleplanes-420-10.y4m").as_slice(),10),(include_bytes!("fixtures/playback-errors/shuffleplanes-444-16.y4m").as_slice(),16)] {
        let mut reader=NativeReader::software(Cursor::new(data),usize::MAX).unwrap();
        let mut frames=Vec::new();
        while let Some(raw)=reader.read_frame_raw().unwrap() {
            match &raw {fvid::playback_native::RawFrame::Planar(p)=>assert_eq!(p.depth,depth),_=>panic!("high depth must retain planar samples")}
            let [w,h]=reader.dimensions();frames.push(VideoGeometry::default().apply(&raw,w,h).unwrap().data);
        }
        assert_eq!(frames.len(),2);
        let mut offset=data.iter().position(|&b|b==b'\n').unwrap()+1;
        for frame in &frames {assert_eq!(&data[offset..offset+6],b"FRAME\n");offset+=6;assert_eq!(&data[offset..offset+frame.len()],frame);offset+=frame.len();}
        reader.rewind().unwrap();
        let raw=reader.read_frame_raw().unwrap().unwrap();let [w,h]=reader.dimensions();assert_eq!(VideoGeometry::default().apply(&raw,w,h).unwrap().data,frames[0]);
        reader.rewind().unwrap();assert!(reader.read_frame().unwrap());assert_eq!(reader.rgb().len(),8*8*3);
    }
}

#[test]
fn declared_y4m_depths_and_layouts_keep_full_width_samples() {
    for depth in [9u8,10,12,14,16] {
        for (layout,sx,sy) in [("420",2,2),("422",2,1),("444",1,1)] {
            let header=format!("YUV4MPEG2 W4 H2 F25:1 Ip C{layout}p{depth}\n");
            let maximum=((1u32<<depth)-1) as u16;
            let samples:Vec<_>=(0..8+2*(4/sx)*(2/sy)).map(|i|if i%2==0 {maximum} else {i as u16}).flat_map(u16::to_le_bytes).collect();
            let mut data=header.as_bytes().to_vec();data.extend_from_slice(b"FRAME\n");data.extend_from_slice(&samples);
            let parsed=fvid::Header::parse(header.as_bytes()).unwrap();assert_eq!(parsed.depth(),depth);assert_eq!(parsed.frame_len().unwrap(),samples.len());
            assert!(fvid::Plan::new(&parsed,Default::default(),usize::MAX).unwrap_err().to_string().contains("byte transform pipeline requires 8-bit"));
            let mut reader=NativeReader::software(Cursor::new(data),usize::MAX).unwrap();
            let raw=reader.read_frame_raw().unwrap().unwrap();assert_eq!(VideoGeometry::default().apply(&raw,4,2).unwrap().data,samples);
            assert!(reader.read_frame_raw().unwrap().is_none());
        }
    }
}

#[test]
fn expression_indices_preserve_owned_fixture_pixels() {
    for (_, source, expected, depth, literal) in CASES {
        let expression = literal.split(':').enumerate()
            .map(|(index, value)| format!("map{index}={value}*2/2"))
            .collect::<Vec<_>>().join(":");
        let mut reader = NativeReader::software(Cursor::new(source), usize::MAX).unwrap();
        let mut actual = Vec::new();
        while let Some(raw) = reader.read_frame_raw().unwrap() {
            let [width, height] = reader.dimensions();
            let mut frame = VideoGeometry::default().apply(&raw, width, height).unwrap();
            apply(ShufflePlanes::parse(&expression).unwrap(), &mut frame, depth).unwrap();
            actual.extend_from_slice(&frame.data);
        }
        assert_eq!(actual, expected);
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/shuffleplanes-444-8.mkv");
    let stats = fvid::media::decode_video_transformed(&path, fvid::media::DecodeTransform {
        shuffleplanes: Some("map0=3/2:map1=default:map2=min".into()),
        ..Default::default()
    }).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.video_frames, 2);
    assert_eq!(ShufflePlanes::parse("map0=3/2:map1=default:map2=min").unwrap().mapping, [2, 1, 0]);
    for invalid in ["map0=n", "map0=1/0", "map0=3.1", "map0=3"] {
        assert!(ShufflePlanes::parse(invalid).is_err(), "{invalid}");
    }
}
