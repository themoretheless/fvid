use fvid::{
    codec::{
        avc::{Pps, Sps},
        avc_slice::{SliceHeader, SliceType},
        config::{AvcConfig, NalUnits},
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
fn compare(file: &[u8], oracle: &[u8], cabac: bool, depth: u8) {
    let mut source = Mp4Reader::open(Cursor::new(file), Default::default()).unwrap();
    let configuration = source.tracks()[0].configuration.clone();
    let config = AvcConfig::parse(&configuration).unwrap();
    let sps = Sps::parse(config.sps[0]).unwrap();
    let pps = Pps::parse(config.pps[0], &sps).unwrap();
    assert_eq!(config.profile, 244);
    assert!(sps.transform_bypass);
    assert_eq!(pps.cabac, cabac);
    assert_eq!(sps.chroma_format, 1);
    assert_eq!(sps.bit_depth_luma, depth);
    assert_eq!(sps.bit_depth_chroma, depth);
    let mut kinds = [false; 3];
    let mut directional_intra = false;
    let mut packet = Vec::new();
    for index in 0..source.tracks()[0].samples.len() {
        source.read_packet(0, index, &mut packet).unwrap();
        let nal = NalUnits::new(&packet, config.length_size)
            .unwrap()
            .map(|n| n.unwrap())
            .find(|n| matches!(n[0] & 31, 1 | 5))
            .unwrap();
        let header = SliceHeader::parse(nal, &sps, &pps).unwrap();
        assert_eq!(header.slice_qp + 6 * i32::from(depth - 8), 0);
        if header.slice_type == SliceType::I {
            let mut cavlc = if pps.cabac {
                None
            } else {
                Some(
                    fvid::codec::avc_macroblock::IntraCavlcReader::new(&header, &sps, &pps, 65536)
                        .unwrap(),
                )
            };
            let mut arithmetic = if pps.cabac {
                Some(
                    fvid::codec::avc_cabac_macroblock::IntraCabacReader::new(
                        &header, &sps, &pps, 65536,
                    )
                    .unwrap(),
                )
            } else {
                None
            };
            loop {
                let block = if let Some(reader) = &mut cavlc {
                    reader.read_macroblock().unwrap()
                } else {
                    arithmetic.as_mut().unwrap().read_macroblock().unwrap()
                };
                let Some(block) = block else { break };
                use fvid::codec::avc_macroblock::IntraLuma;
                if matches!(block.luma, IntraLuma::Pcm { .. }) {
                    assert_eq!(block.qp, 0);
                } else {
                    assert_eq!(block.qp + 6 * i32::from(depth - 8), 0);
                }
                directional_intra |= match block.luma {
                    IntraLuma::Blocks4(modes) => {
                        modes.iter().any(|m| (*m as u8) < 2)
                            && block.luma_levels.iter().flatten().any(|v| *v != 0)
                    }
                    IntraLuma::Blocks8 { modes, levels } => {
                        modes.iter().any(|m| (*m as u8) < 2)
                            && levels.iter().flatten().any(|v| *v != 0)
                    }
                    IntraLuma::Block16(mode) => {
                        mode < 2
                            && block
                                .luma_dc
                                .iter()
                                .chain(block.luma_levels.iter().flatten())
                                .any(|v| *v != 0)
                    }
                    IntraLuma::Pcm { .. } => false,
                };
            }
        }
        let index = match header.slice_type {
            SliceType::I => 0,
            SliceType::P => 1,
            SliceType::B => 2,
            _ => panic!("unexpected slice"),
        };
        kinds[index] = true;
    }
    assert_eq!(kinds, [true, true, false]);
    assert!(
        directional_intra,
        "fixture must exercise intra residual DPCM"
    );
    let mut reader =
        fvid::playback_mp4::Mp4VideoReader::open_software(Cursor::new(file), Default::default(), 16 << 20)
            .unwrap();
    assert!(!reader.hardware_accelerated());
    for _ in 0..2 {
        let mut actual = Vec::new();
        let mut frames = 0;
        while let Some(frame) = reader.read_frame().unwrap() {
            for sample in frame
                .picture
                .y
                .iter()
                .chain(&frame.picture.cb)
                .chain(&frame.picture.cr)
            {
                if depth == 8 {
                    actual.push(*sample as u8);
                } else {
                    actual.extend_from_slice(&sample.to_le_bytes());
                }
            }
            frames += 1;
        }
        assert_eq!(frames, 8);
        assert_eq!(actual, oracle);
        reader.rewind();
    }
}

#[test]
fn lossless_cabac_matches_original_samples_and_rewind() {
    compare(
        include_bytes!("fixtures/playback-errors/avc-bypass-lossless.mp4"),
        include_bytes!("fixtures/playback-errors/avc-bypass-lossless.yuv"),
        true,
        8,
    );
}
#[test]
fn lossless_cavlc_matches_original_samples_and_rewind() {
    compare(
        include_bytes!("fixtures/playback-errors/avc-bypass-cavlc.mp4"),
        include_bytes!("fixtures/playback-errors/avc-bypass-cavlc.yuv"),
        false,
        8,
    );
}

#[test]
fn lossless_ten_bit_matches_original_samples_and_rewind() {
    compare(
        include_bytes!("fixtures/playback-errors/avc-bypass-main10.mp4"),
        include_bytes!("fixtures/playback-errors/avc-bypass-main10.yuv"),
        true,
        10,
    );
}

#[test]
fn lossless_ten_bit_cavlc_matches_original_samples_and_rewind() {
    compare(
        include_bytes!("fixtures/playback-errors/avc-bypass-cavlc10.mp4"),
        include_bytes!("fixtures/playback-errors/avc-bypass-cavlc10.yuv"),
        false,
        10,
    );
}

#[test]
fn ten_bit_lossless_camera_bridge_preserves_native_pixels_and_backward_seek() {
    use fvid::{
        playback_native::NativeReader,
        virtual_camera::{CameraTick, LatestFrame, NativeCameraSource},
    };
    for file in [
        include_bytes!("fixtures/playback-errors/avc-bypass-main10.mp4").as_slice(),
        include_bytes!("fixtures/playback-errors/avc-bypass-cavlc10.mp4").as_slice(),
    ] {
        let mut reader = NativeReader::software(Cursor::new(file), 16 << 20).unwrap();
        let mut expected = Vec::new();
        while reader.read_frame().unwrap() {
            assert_eq!(reader.dimensions(), [64, 64]);
            let (start, _, scale) = reader.frame_interval().unwrap();
            let position = (start * 1_000_000_000).div_ceil(u128::from(scale)) as u64;
            let pixels: Vec<u8> = reader
                .rgb()
                .as_chunks::<3>().0.iter()
                .flat_map(|p| [p[2], p[1], p[0], 255])
                .collect();
            expected.push((position, pixels));
        }
        assert_eq!(expected.len(), 8);
        let mut source =
            NativeCameraSource::new(NativeReader::software(Cursor::new(file), 16 << 20).unwrap());
        let output = LatestFrame::new(64, 64, 64 * 64 * 4).unwrap();
        let mut pixels = vec![0; 64 * 64 * 4];
        for (sequence, (position, reference)) in
            expected.iter().chain(expected.iter().take(1)).enumerate()
        {
            let tick = CameraTick {
                sequence: sequence as u64,
                host_time_ns: sequence as u64 + 1,
                media_time_ns: *position,
            };
            assert!(source.publish(tick, &output).unwrap());
            assert_eq!(output.copy_latest(None, &mut pixels).unwrap(), Some(tick));
            assert_eq!(&pixels, reference);
        }
    }
}

#[test]
fn compressed_lossless_camera_loops_all_frames() {
    use fvid::{
        playback_native::NativeReader,
        virtual_camera::{CameraEndBehavior, CameraTick, LatestFrame, NativeCameraSource},
    };
    let file = include_bytes!("fixtures/playback-errors/avc-bypass-main10.mp4").as_slice();
    let mut reader = NativeReader::software(Cursor::new(file), 16 << 20).unwrap();
    let mut expected = Vec::new();
    let mut duration = 0;
    while reader.read_frame().unwrap() {
        let (start, end, scale) = reader.frame_interval().unwrap();
        duration = (end * 1_000_000_000).div_ceil(u128::from(scale)) as u64;
        expected.push((
            (start * 1_000_000_000).div_ceil(u128::from(scale)) as u64,
            reader
                .rgb()
                .as_chunks::<3>().0.iter()
                .flat_map(|p| [p[2], p[1], p[0], 255])
                .collect::<Vec<_>>(),
        ));
    }
    let mut source =
        NativeCameraSource::new(NativeReader::software(Cursor::new(file), 16 << 20).unwrap())
            .with_end_behavior(CameraEndBehavior::Loop);
    let output = LatestFrame::new(64, 64, 64 * 64 * 4).unwrap();
    let mut pixels = vec![0; 64 * 64 * 4];
    for cycle in 0..3 {
        for (index, (position, reference)) in expected.iter().enumerate() {
            let sequence = cycle * expected.len() + index;
            let tick = CameraTick {
                sequence: sequence as u64,
                host_time_ns: sequence as u64 + 1,
                media_time_ns: *position + cycle as u64 * duration,
            };
            assert!(source.publish(tick, &output).unwrap());
            assert_eq!(output.copy_latest(None, &mut pixels).unwrap(), Some(tick));
            assert_eq!(&pixels, reference);
        }
    }
}
