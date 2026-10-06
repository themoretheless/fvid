use fvid::{codec::{config::HevcConfig, hevc_decoder::HevcDecoder, hevc_sps::Sps}, container::mp4::Mp4Reader};
use std::io::Cursor;

fn samples() -> [(&'static [u8], &'static [u8], u8, [u32;2]); 6] {
    macro_rules! sample { ($stem:literal, $depth:literal, $width:literal, $height:literal) => {
        (include_bytes!(concat!("fixtures/playback-errors/", $stem, ".mp4")).as_slice(),
         include_bytes!(concat!("fixtures/playback-errors/", $stem, ".yuv")).as_slice(), $depth, [$width,$height])
    }; }
    [sample!("hevc-chroma422-filtered-rext8", 8, 64, 64),
     sample!("hevc-chroma422-filtered-rext10", 10, 64, 64),
     sample!("hevc-chroma422-filtered-rext12", 12, 64, 64),
     sample!("hevc-chroma422-wpp-rext12", 12, 64, 64),
     sample!("hevc-chroma422-mixed-tiles-rext12", 12, 64, 64),
     sample!("hevc-chroma422-parallel-rext12", 12, 128, 96)]
}

#[test]
fn chroma422_metadata_and_first_picture_are_admitted() {
    for (sample_index, (source, oracle, depth, [width,height])) in samples().into_iter().enumerate() {
        let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
        let config = input.tracks()[0].configuration.clone();
        let parsed = HevcConfig::parse(&config).unwrap();
        let sps = Sps::parse(parsed.arrays.iter().find(|a| a.nal_type == 33).unwrap().units[0], 16 << 20).unwrap();
        assert_eq!(sps.chroma_format, 2);
        assert_eq!(sps.depth, [depth; 2]);
        assert_eq!(sps.dimensions, [width,height]);
        assert!(sps.sao);
        assert_eq!(input.tracks()[0].samples.len(), 3);
        assert_eq!(oracle.len(), 3 * width as usize * height as usize * 2 * if depth == 8 { 1 } else { 2 });
        let mut decoder = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let pps = decoder.parameters().1;
        assert!(!pps.deblocking.disabled);
        assert_eq!(pps.entropy_sync, matches!(sample_index,3|5));
        assert_eq!(pps.tiles.is_some(), sample_index == 4);
        if sample_index == 4 {
            assert!(pps.dependent_slices);
            assert_eq!(pps.tiles.as_ref().unwrap().column_widths.len(),2);
            let headers = decoder.slice_headers(&packet).unwrap();
            assert!(headers.len() > 1);
            assert!(headers.iter().any(|s| s.dependent));
            assert!(headers.iter().filter(|s| !s.dependent).count() > 1);
        }
        if sample_index == 5 {
            assert!(width * height >= 128 * 96);
            assert!(!pps.constrained_intra);
        }
        let frame = decoder.decode_packet(&packet).unwrap().unwrap();
        assert_eq!(frame.picture.planes.each_ref().map(|p| p.dimensions()),
            [[width as usize,height as usize],[width as usize/2,height as usize],[width as usize/2,height as usize]]);
    }
}

#[test]
fn chroma422_software_playback_matches_hm_and_rewind() {
    for (source,oracle,depth,[width,height]) in samples() {
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(source),Default::default(),16 << 20).unwrap();
        assert!(!reader.hardware_accelerated());
        for _ in 0..2 {
            let mut actual = Vec::new();
            let mut count = 0;
            while let Some(frame) = reader.read_frame().unwrap() {
                assert_eq!([frame.picture.y.len(),frame.picture.cb.len(),frame.picture.cr.len()],
                    [width as usize*height as usize,width as usize*height as usize/2,width as usize*height as usize/2]);
                for plane in [&frame.picture.y,&frame.picture.cb,&frame.picture.cr] {
                    for &v in plane {
                        if depth == 8 { actual.push(v as u8); }
                        else { actual.extend_from_slice(&v.to_le_bytes()); }
                    }
                }
                let packed = frame.packed.as_ref().expect("HEVC 422 needs explicit display geometry");
                assert_eq!(packed.depth,depth);
                assert_eq!(packed.frame.subsampling,Some([2,1]));
                assert_eq!([packed.frame.width,packed.frame.height],[width as usize,height as usize]);
                let frame_bytes = width as usize*height as usize*2*if depth==8 {1}else{2};
                assert_eq!(packed.frame.data,&oracle[count*frame_bytes..(count+1)*frame_bytes]);
                count += 1;
            }
            assert_eq!(count,3);
            assert!(actual == oracle,"software playback differs at depth {depth}");
            reader.rewind();
        }
    }
}

#[test]
fn chroma422_every_sample_and_reset_match_hm() {
    for (source, oracle, depth, [width,height]) in samples() {
        let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
        let mut decoder = HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            for index in 0..3 {
                let mut packet = Vec::new();
                input.read_packet(0, index, &mut packet).unwrap();
                let frame = decoder.decode_packet(&packet).unwrap().unwrap();
                assert_eq!(frame.picture.planes.each_ref().map(|p| p.samples().len()), [width as usize*height as usize,width as usize*height as usize/2,width as usize*height as usize/2]);
                for plane in &frame.picture.planes {
                    for &v in plane.samples() {
                        if depth == 8 { actual.push(v as u8); } else { actual.extend_from_slice(&v.to_le_bytes()); }
                    }
                }
            }
            assert_eq!(actual.len(), oracle.len());
            assert!(actual == oracle, "first mismatch {:?}", actual.iter().zip(oracle).position(|(a,b)| a != b));
            decoder.reset();
        }
    }
}
