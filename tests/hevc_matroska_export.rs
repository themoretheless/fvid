//! Container HEVC export must preserve independently authored display samples.
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors") }
#[test]
fn hevc_matroska_lossless_export_preserves_display_codes_and_depth() {
    for manifest in ["hevc-identity-rgb.json", "hevc-matroska-formats.json"] {
        let m: serde_json::Value = serde_json::from_slice(&std::fs::read(root().join(manifest)).unwrap()).unwrap();
        assert_eq!(m["cases"].as_array().unwrap().len(), if manifest == "hevc-identity-rgb.json" {80} else {112});
        for (index, c) in m["cases"].as_array().unwrap().iter().enumerate() {
            let output = std::env::temp_dir().join(format!("fvid-hevc-mkv-{}-{manifest}-{index}.mkv",std::process::id()));
            fvid_media::transcode_lossless(&root().join(c["file"].as_str().unwrap()), &output,
                fvid::media_info::LosslessTransform { horizontal_flip: true, ..Default::default() }, &Default::default()).unwrap();
            let mut reader = fvid_media::owned_webm::WebmReader::open(Cursor::new(std::fs::read(&output).unwrap()), Default::default()).unwrap();
            reader.scan_all().unwrap();
            let track = reader.tracks[0].clone();
            let (w,h)=(track.width as usize,track.height as usize);
            let mut size = [c["size"][0].as_u64().unwrap()-c["crop"][0].as_u64().unwrap()-c["crop"][2].as_u64().unwrap(), c["size"][1].as_u64().unwrap()-c["crop"][1].as_u64().unwrap()-c["crop"][3].as_u64().unwrap()];
            if matches!(c["rotation"].as_u64().unwrap(),90|270){size.swap(0,1);}
            assert_eq!([w as u64,h as u64],size);
            let depth = c["depth"].as_u64().unwrap() as u8;
            let unit = if depth == 8 {1} else {2};
            let rgb = manifest == "hevc-identity-rgb.json";
            let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(w,h,&track.codec_private,32<<20).unwrap();
            let mut player = fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(&output).unwrap()),32<<20).unwrap();
            assert_eq!(reader.packets.len(),c["frames"].as_array().unwrap().len());
            for (i,f) in c["frames"].as_array().unwrap().iter().enumerate() {
                let actual=decoder.decode(&reader.read_packet(i).unwrap()).unwrap();
                assert_eq!(actual.depth,depth);
                let source=std::fs::read(root().join(f["display"].as_str().unwrap())).unwrap();
                let mut expected=Vec::new();
                if rgb {
                    assert_eq!(actual.frame.subsampling,None);
                    for row in source.chunks_exact(w*3*unit) {for pixel in row.chunks_exact(3*unit).rev(){expected.extend_from_slice(pixel);}}
                    assert_eq!(track.colour.full_range,c["full_range"].as_bool().unwrap());
                } else {
                    let mono=c["chroma_format"].as_u64().unwrap()==0;
                    let mut sub=[c["subsampling"][0].as_u64().unwrap() as usize,c["subsampling"][1].as_u64().unwrap() as usize];
                    if matches!(c["rotation"].as_u64().unwrap(),90|270){sub.swap(0,1);}
                    let mut offset=0;
                    for component in 0..3 {
                        if mono && component>0 {
                            let value=1u16<<(depth-1);
                            for _ in 0..w*h {if unit==1{expected.push(value as u8)}else{expected.extend(value.to_le_bytes())}}
                            continue;
                        }
                        let [sx,sy]=if component==0{[1,1]}else{sub};
                        let (pw,ph)=(w.div_ceil(sx),h.div_ceil(sy));
                        for row in source[offset..offset+pw*ph*unit].chunks_exact(pw*unit){for sample in row.chunks_exact(unit).rev(){expected.extend_from_slice(sample);}}
                        offset+=pw*ph*unit;
                    }
                }
                assert_eq!(actual.frame.data,expected,"{} frame {i}",c["file"]);
                assert_eq!(reader.packets[i].pts_ns,i as i64*1_000_000_000/c["rate"].as_i64().unwrap());
                if rgb {
                    let fvid::playback_native::RawFrame::Planar(p)=player.read_frame_raw().unwrap().unwrap() else {panic!("RGB output payload")};
                    let source=std::fs::read(root().join(f["display8"].as_str().unwrap())).unwrap();
                    let expected:Vec<u8>=source.chunks_exact(w*3).flat_map(|row|row.chunks_exact(3).rev().flatten().copied()).collect();
                    let mut rgb8=Vec::new();p.to_rgb_on_black(&mut rgb8,32<<20).unwrap();
                    assert_eq!(rgb8,expected,"Matroska export range presentation");
                }
            }
            std::fs::remove_file(output).unwrap();
        }
    }
}
