use fvid::{
    codec::{
        avc::{Pps, Sps},
        avc_scaling::ScalingMatrices,
        config::AvcConfig,
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
fn compare(file: &[u8], oracle: &[u8], require_eight: bool) {
    let mut source = Mp4Reader::open(Cursor::new(file), Default::default()).unwrap();
    let samples = &source.tracks()[0].samples;
    assert_eq!(samples.len(), 8);
    let presentation: Vec<_> = (0..8).map(|i| samples.get(i).unwrap().pts).collect();
    assert!(
        presentation.windows(2).any(|pair| pair[1] < pair[0]),
        "fixture must retain B-frame reordering"
    );
    for i in 0..8 {
        let sample = samples.get(i).unwrap();
        assert_eq!(sample.dts, i as u64);
        assert_eq!(sample.duration, 1);
    }
    let mut sorted = presentation;
    sorted.sort();
    assert_eq!(sorted, (0..8).collect::<Vec<_>>());
    let configuration = source.tracks()[0].configuration.clone();
    let config = AvcConfig::parse(&configuration).unwrap();
    let sps = Sps::parse(config.sps[0]).unwrap();
    let pps = Pps::parse(config.pps[0], &sps).unwrap();
    assert!(sps.scaling_lists.is_some() || pps.scaling_lists.is_some());
    assert!(pps.transform_8x8);
    let scaling = ScalingMatrices::new(&sps, &pps).unwrap();
    assert_ne!(scaling.four, [[16; 16]; 6]);
    assert_ne!(scaling.eight, [[16; 64]; 2]);
    let mut packet = Vec::new();
    source.read_packet(0, 0, &mut packet).unwrap();
    let nal = fvid::codec::config::NalUnits::new(&packet, config.length_size)
        .unwrap()
        .map(|n| n.unwrap())
        .find(|n| n[0] & 31 == 5)
        .unwrap();
    let header = fvid::codec::avc_slice::SliceHeader::parse(nal, &sps, &pps).unwrap();
    let mut entropy =
        fvid::codec::avc_cabac_macroblock::IntraCabacReader::new(&header, &sps, &pps, 65536)
            .unwrap();
    let mut eight = false;
    while let Some(block) = entropy.read_macroblock().unwrap() {
        if let fvid::codec::avc_macroblock::IntraLuma::Blocks8 { levels, .. } = block.luma {
            eight |= levels.iter().flatten().any(|v| *v != 0);
        }
    }
    if require_eight {
        assert!(
            eight,
            "fixture must reconstruct nonzero 8x8 intra residuals"
        );
    }
    let mut reader =
        fvid::playback_mp4::Mp4VideoReader::open(Cursor::new(file), Default::default(), 16 << 20)
            .unwrap();
    assert!(!reader.hardware_accelerated());
    for _ in 0..2 {
        let mut actual = Vec::new();
        let mut frames = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            actual.extend(
                frame
                    .picture
                    .y
                    .iter()
                    .chain(&frame.picture.cb)
                    .chain(&frame.picture.cr)
                    .map(|v| *v as u8),
            );
            frames += 1;
        }
        assert_eq!(frames, 8);
        assert_eq!(actual, oracle);
        reader.rewind();
    }
}

#[test]
fn jvt_scaling_ipb_matches_saved_independent_yuv_and_rewind() {
    compare(
        include_bytes!("fixtures/playback-errors/avc-scaling-jvt.mp4"),
        include_bytes!("fixtures/playback-errors/avc-scaling-jvt.yuv"),
        false,
    );
}
#[test]
fn custom_scaling_ipb_matches_saved_independent_yuv_and_rewind() {
    compare(
        include_bytes!("fixtures/playback-errors/avc-scaling-custom.mp4"),
        include_bytes!("fixtures/playback-errors/avc-scaling-custom.yuv"),
        true,
    );
}
#[test]
fn matrix_scan_defaults_and_sps_pps_fallback_are_distinct() {
    use fvid::codec::avc::ScalingList;
    let source = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/avc-scaling-jvt.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let config = AvcConfig::parse(&source.tracks()[0].configuration).unwrap();
    let mut sps = Sps::parse(config.sps[0]).unwrap();
    let mut pps = Pps::parse(config.pps[0], &sps).unwrap();
    sps.scaling_lists = None;
    pps.scaling_lists = None;
    let flat = ScalingMatrices::new(&sps, &pps).unwrap();
    assert_eq!(flat.four, [[16; 16]; 6]);
    assert_eq!(flat.eight, [[16; 64]; 2]);
    sps.scaling_lists = Some(vec![None; 8]);
    let defaults = ScalingMatrices::new(&sps, &pps).unwrap();
    assert_eq!(
        defaults.four[0],
        [
            6, 13, 20, 28, 13, 20, 28, 32, 20, 28, 32, 37, 28, 32, 37, 42
        ]
    );
    assert_eq!(defaults.four[1], defaults.four[0]);
    assert_eq!(defaults.four[5], defaults.four[3]);
    sps.scaling_lists.as_mut().unwrap()[0] = Some(ScalingList::Explicit((1..=16).collect()));
    sps.scaling_lists.as_mut().unwrap()[1] = Some(ScalingList::Explicit(vec![27; 16]));
    sps.scaling_lists.as_mut().unwrap()[6] = Some(ScalingList::Explicit((1..=64).collect()));
    let sequence = ScalingMatrices::new(&sps, &pps).unwrap();
    assert_eq!(sequence.four[0][4], 3);
    assert_eq!(sequence.four[0][15], 16);
    assert_eq!(sequence.four[1], [27; 16]);
    assert_eq!(sequence.four[2], [27; 16]);
    assert_eq!(sequence.eight[0][8], 3);
    assert_eq!(sequence.eight[0][63], 64);
    pps.scaling_lists = Some(vec![None; 8]);
    pps.scaling_lists.as_mut().unwrap()[2] = Some(ScalingList::Default);
    pps.scaling_lists.as_mut().unwrap()[7] = Some(ScalingList::Explicit(vec![33; 64]));
    let picture = ScalingMatrices::new(&sps, &pps).unwrap();
    assert_eq!(picture.four[0], sequence.four[0]);
    assert_eq!(picture.four[1], sequence.four[0]);
    assert_eq!(picture.four[2], defaults.four[0]);
    assert_eq!(picture.four[3], sequence.four[3]);
    assert_eq!(picture.eight[0], sequence.eight[0]);
    assert_eq!(picture.eight[1], [33; 64]);
    pps.scaling_lists.as_mut().unwrap()[0] = Some(ScalingList::Explicit(vec![0; 16]));
    assert!(ScalingMatrices::new(&sps, &pps).is_err());
    pps.scaling_lists.as_mut().unwrap()[0] = Some(ScalingList::Explicit(vec![16; 15]));
    assert!(ScalingMatrices::new(&sps, &pps).is_err());
}
