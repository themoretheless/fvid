use fvid::{codec::{config::HevcConfig, hevc_decoder::HevcDecoder, hevc_sps::Sps},container::mp4::Mp4Reader};
use std::io::Cursor;

fn samples() -> [(&'static [u8], &'static [u8],u8);4] {
    macro_rules! sample { ($stem:literal,$depth:literal) => {
        (include_bytes!(concat!("fixtures/playback-errors/",$stem,".mp4")).as_slice(),
         include_bytes!(concat!("fixtures/playback-errors/",$stem,".yuv")).as_slice(),$depth)
    }; }
    [sample!("hevc-cross-component-rext8",8),sample!("hevc-cross-component-rext10",10),sample!("hevc-cross-component-rext12",12),sample!("hevc-cross-component-parallel-rext12",12)]
}

#[test]
fn cross_component_pps_and_picture_are_admitted() {
    for (sample_index,(source,oracle,depth)) in samples().into_iter().enumerate() {
        let mut input = Mp4Reader::open(Cursor::new(source),Default::default()).unwrap();
        assert_eq!(input.tracks()[0].samples.len(),3);
        let config = &input.tracks()[0].configuration;
        let parsed = HevcConfig::parse(config).unwrap();
        let sps = Sps::parse(parsed.arrays.iter().find(|a|a.nal_type==33).unwrap().units[0],16 << 20).unwrap();
        assert_eq!(sps.chroma_format,3);
        assert_eq!(sps.depth,[depth;2]);
        let dimensions = if sample_index == 3 {[128,96]} else {[64,64]};
        assert_eq!(sps.dimensions,dimensions);
        assert_eq!(oracle.len(),3*dimensions[0] as usize*dimensions[1] as usize*3*if depth==8 {1} else {2});
        let pps = parsed.arrays.iter().find(|a|a.nal_type==34).unwrap().units[0];
        let pps = fvid::codec::hevc_pps::Pps::parse(pps,&sps,16 << 20).unwrap();
        assert!(pps.cross_component_prediction);
        assert_eq!(pps.entropy_sync,sample_index == 3);
        assert!(!pps.constrained_intra);
        let mut decoder = HevcDecoder::from_configuration(config,16 << 20).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0,0,&mut packet).unwrap();
        assert!(decoder.decode_packet(&packet).unwrap().is_some());
    }
}

#[test]
fn cross_component_every_sample_and_reset_match_hm() {
    for (source,oracle,depth) in samples() {
        let mut input = Mp4Reader::open(Cursor::new(source),Default::default()).unwrap();
        let mut decoder = HevcDecoder::from_configuration(&input.tracks()[0].configuration,16 << 20).unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            for index in 0..3 {
                let mut packet = Vec::new();
                input.read_packet(0,index,&mut packet).unwrap();
                let frame = decoder.decode_packet(&packet).unwrap().unwrap();
                for plane in &frame.picture.planes { for &v in plane.samples() {
                    if depth==8 {actual.push(v as u8);} else {actual.extend_from_slice(&v.to_le_bytes());}
                }}
            }
            assert_eq!(actual.len(),oracle.len());
            assert!(actual == oracle,"first mismatch {:?}",actual.iter().zip(oracle).position(|(a,b)|a!=b));
            decoder.reset();
        }
    }
}

#[test]
fn cross_component_software_playback_matches_hm_and_rewind() {
    for (source,oracle,depth) in samples() {
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(source),Default::default(),16 << 20).unwrap();
        assert!(!reader.hardware_accelerated());
        for _ in 0..2 {
            let mut actual = Vec::new();
            let mut frames = 0;
            while let Some(frame) = reader.read_frame().unwrap() {
                for plane in [&frame.picture.y,&frame.picture.cb,&frame.picture.cr] {
                    for &v in plane {
                        if depth == 8 {actual.push(v as u8);} else {actual.extend_from_slice(&v.to_le_bytes());}
                    }
                }
                frames += 1;
            }
            assert_eq!(frames,3);
            assert!(actual == oracle,"software cross-component pixels differ at depth {depth}");
            reader.rewind();
        }
    }
}
