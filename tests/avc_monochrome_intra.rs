use fvid::codec::avc_decoder::AvcDecoder;
#[test]
fn monochrome_compressed_intra_prediction() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/avc-monochrome-intra");
    let cases:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in cases.as_array().unwrap() {
        let name=case["name"].as_str().unwrap();
        let config=std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let packet=std::fs::read(root.join(format!("{name}.packet"))).unwrap();
        let mut decoder=AvcDecoder::new(&config,1<<20).unwrap();
        let golden=std::fs::read(root.join(format!("{name}.y16"))).unwrap();
        let samples:Vec<u16>=golden.chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
        for _ in 0..2 {
            let picture=decoder.decode_order(&packet).unwrap_or_else(|e|panic!("{name}: {e}")).unwrap();
            assert_eq!(picture.y,samples,"{name}");
            assert!(picture.cb.is_empty()&&picture.cr.is_empty());
            assert_eq!(u64::from(picture.bit_depth),case["depth"].as_u64().unwrap());
            decoder.reset();
        }
        let data=std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
        let mut reader=fvid::playback::video::mp4::Mp4VideoReader::open(std::io::Cursor::new(data),fvid::container::mp4::Limits::default(),16<<20).unwrap();
        for _ in 0..2 {
            let frame=reader.read_frame().unwrap().unwrap();
            let packed=frame.packed.unwrap();
            let depth=case["depth"].as_u64().unwrap()as u8;
            assert_eq!(packed.depth,depth);
            assert_eq!(packed.frame.subsampling,Some([2,2]));
            let mut expected=Vec::new();
            for sample in samples.iter().copied().chain(std::iter::repeat_n(1u16<<(depth-1),256)) {
                if depth==8 {expected.push(sample as u8);}else{expected.extend_from_slice(&sample.to_le_bytes());}
            }
            assert_eq!(packed.frame.data,expected,"MP4 {name}");
            assert!(reader.read_frame().unwrap().is_none());reader.rewind();
        }
    }
}
