use fvid::codec::avc_decoder::AvcDecoder;
use std::path::{Path,PathBuf};
fn fixtures()->(PathBuf,serde_json::Value,Vec<u16>) {
    let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/avc-pcm-formats");
    let manifest=serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let golden=std::fs::read(root.join("samples.y16")).unwrap().chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
    (root,manifest,golden)
}
fn shapes(format:u64)->[(usize,usize);3] {
    match format {0=>[(32,16),(0,0),(0,0)],2=>[(32,16),(16,16),(16,16)],3=>[(32,16);3],_=>panic!("unexpected format")}
}
fn visible(case:&serde_json::Value,frame:&serde_json::Value,golden:&[u16],packed:bool)->Vec<u8> {
    let fmt=case["format"].as_u64().unwrap();let dimensions=shapes(fmt);
    let crop:Vec<usize>=case["crop"].as_array().unwrap().iter().map(|v|v.as_u64().unwrap()as usize).collect();
    let depths=[case["luma_depth"].as_u64().unwrap()as u8,case["chroma_depth"].as_u64().unwrap()as u8];
    let output_depth=if fmt==0 {depths[0]}else{depths[0].max(depths[1])};
    let sub=if fmt==2 {[2,1]}else if fmt==0 {[2,2]}else{[1,1]};
    let mut output=Vec::new();let mut at=frame["offset"].as_u64().unwrap()as usize;
    for c in 0..3 {
        if c!=0&&fmt==0 {
            if packed {
                let w=(32-crop[0]-crop[1]).div_ceil(2);let h=(16-crop[2]-crop[3]).div_ceil(2);
                for _ in 0..w*h {let v=1u16<<(output_depth-1);if output_depth==8 {output.push(v as u8);}else{output.extend_from_slice(&v.to_le_bytes());}}
            }
            continue;
        }
        let [sx,sy]=if c==0 {[1,1]}else{sub};
        let (stride,rows)=dimensions[c];let count=stride*rows;
        let plane=&golden[at..at+count];at+=count;
        let width=(32-crop[0]-crop[1])/sx;let height=(16-crop[2]-crop[3])/sy;
        let source_depth=depths[usize::from(c!=0)];let depth=if packed {output_depth}else{source_depth};
        for y in 0..height {for x in 0..width {
            let value=plane[(crop[2]/sy+y)*stride+crop[0]/sx+x]<<(depth-source_depth);
            if depth==8 {output.push(value as u8);}else{output.extend_from_slice(&value.to_le_bytes());}
        }}
    }
    output
}
#[test]
fn pcm_decodes_monochrome_422_444_with_native_depth_crop_and_reset() {
    let (root,manifest,golden)=fixtures();assert_eq!(manifest["cases"].as_array().unwrap().len(),18);
    for case in manifest["cases"].as_array().unwrap() {
        let mut decoder=AvcDecoder::new(&std::fs::read(root.join(case["configuration"].as_str().unwrap())).unwrap(),1<<20).unwrap();
        for _ in 0..2 {
            for frame in case["frames"].as_array().unwrap() {
                let packet=std::fs::read(root.join(frame["packet"].as_str().unwrap())).unwrap();
                let picture=decoder.decode_order(&packet).unwrap_or_else(|e|panic!("{}: {e}",case["file"])).unwrap();
                assert_eq!(u64::from(picture.chroma_format().unwrap()),case["format"].as_u64().unwrap());
                let mut at=frame["offset"].as_u64().unwrap()as usize;
                for (c,plane)in [&picture.y,&picture.cb,&picture.cr].into_iter().enumerate() {
                    let (w,h)=shapes(case["format"].as_u64().unwrap())[c];assert_eq!(plane.len(),w*h);
                    assert_eq!(plane,&golden[at..at+w*h],"{} c{c}",case["file"]);at+=w*h;
                }
                let mut bytes=Vec::new();picture.write_planar(&mut bytes).unwrap();assert_eq!(bytes,visible(case,frame,&golden,false));
            }
            decoder.reset();
        }
    }
}
#[test]
fn non420_pcm_plane_budget_covers_actual_sample_storage() {
    let (root,manifest,_)=fixtures();
    for case in manifest["cases"].as_array().unwrap() {
        let format=case["format"].as_u64().unwrap();let storage=shapes(format).iter().map(|(w,h)|w*h*2).sum::<usize>();
        let motion=fvid::codec::avc_reference_motion::ReferenceMotionField::storage_bytes(32,16).unwrap();
        let config=std::fs::read(root.join(case["configuration"].as_str().unwrap())).unwrap();
        let packet=std::fs::read(root.join(case["frames"][0]["packet"].as_str().unwrap())).unwrap();
        let required=2*storage+motion;
        let mut decoder=AvcDecoder::new(&config,required-1).unwrap();let error=decoder.decode_order(&packet).unwrap_err();
        assert!(error.to_string().contains("budget"),"{error}");
        let mut decoder=AvcDecoder::new(&config,required).unwrap();assert!(decoder.decode_order(&packet).unwrap().is_some());
    }
}
#[test]
fn mp4_playback_preserves_native_format_samples_and_rewind() {
    use fvid::{container::mp4::Limits,playback::video::mp4::Mp4VideoReader};
    let (root,manifest,golden)=fixtures();
    for case in manifest["cases"].as_array().unwrap() {
        let data=std::fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
        let mut reader=Mp4VideoReader::open(std::io::Cursor::new(data),Limits::default(),16<<20).unwrap();
        for _ in 0..2 {
            let mut count=0;
            while let Some(decoded)=reader.read_frame().unwrap_or_else(|e|panic!("{}: {e}",case["file"])) {
                let packed=decoded.packed.as_ref().expect("non420 AVC must expose explicit packed geometry");
                let fmt=case["format"].as_u64().unwrap();assert_eq!(packed.frame.subsampling,Some(if fmt==2 {[2,1]}else if fmt==3 {[1,1]}else{[2,2]}));
                let luma_depth=case["luma_depth"].as_u64().unwrap();
                let chroma_depth=case["chroma_depth"].as_u64().unwrap();
                assert_eq!(u64::from(packed.depth),if fmt==0 {luma_depth}else{luma_depth.max(chroma_depth)});
                assert_eq!(packed.frame.data,visible(case,&case["frames"][count],&golden,true));count+=1;
            }
            assert_eq!(count,2);reader.rewind();
        }
    }
}
