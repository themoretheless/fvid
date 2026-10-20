use fvid::{
    codec::{bits::BitReader, hevc_3d_syntax::SpsExtension},
    container::mp4::Limits,
    playback::video::mp4::LayeredMp4VideoReader,
};
use std::io::Cursor;
#[test]
fn coded_dbbp_movies_decode_both_partitions_and_flag_values() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("dbbp.json")).unwrap()).unwrap();
    let bytes = std::fs::read(root.join("dbbp-syntax.bin")).unwrap();
    let mut bits = BitReader::new(&bytes);
    let mut extension = SpsExtension::read(
        &mut bits,
        manifest["syntax_bits"].as_u64().unwrap() as usize,
        [3, 5],
    )
    .unwrap();
    assert!(extension.depth_based_partition);
    extension.depth_based_partition = false;
    assert!(
        !extension.active(),
        "DBBP must be the sole active SPS tool in this reproducer"
    );
    for cu in [16usize,32,64] {
    let manifest=if cu!=32 {serde_json::from_slice(&std::fs::read(root.join(format!("dbbp-cu{cu}.json"))).unwrap()).unwrap()}else{manifest.clone()};
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 12);
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(std::fs::read(root.join(name)).unwrap()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        let vertical=case["vertical"].as_bool().unwrap();
        let enabled=case["enabled"].as_bool().unwrap();
        for _ in 0..2 {
        let mut count=0;
        while let Some(frame)=reader.read_frame().unwrap() {
            count+=1;
            if frame.decoded.layer_id!=5 || frame.decoded.poc!=1 {continue;}
            for c in 0..3 {
                let size=case["size"].as_u64().unwrap()as usize;
                let side=if c==0 {size}else{size/2};
                let scale=if c==0 {1}else{2};
                let block=cu/scale;
                let depth=|x:usize,y:usize| (7*x+11*y+37)%256;
                for by in (0..side).step_by(block) {for bx in (0..side).step_by(block) {
                let ox=bx*scale;let oy=by*scale;
                let threshold=(depth(ox,oy)+depth(ox+cu-1,oy)+depth(ox,oy+cu-1)+depth(ox+cu-1,oy+cu-1))/4;
                let label=|x:usize,y:usize| depth(ox+x,oy+y)>threshold;
                let combined=|x:usize,y:usize| {
                    let second=if enabled {label(x*scale,y*scale)!=label(0,0)}else{if vertical{x>=block/2}else{y>=block/2}};
                    let x=x+bx;let y=y+by;
                    let base=if c==0 {(3*x+5*y+17)%256}else{(13*x+7*y+29+83*(c-1))%256};
                    if second {base} else if c==0 {(base*2).saturating_sub(40).min(255)}else{(base+if c==1{71}else{113})%256}
                };
                for y in 0..block {for x in 0..block {
                    let (px,py,nx,ny)=if vertical {(x.saturating_sub(1),y,(x+1).min(block-1),y)}else{(x,y.saturating_sub(1),x,(y+1).min(block-1))};
                    let next=if vertical{label((x*scale+scale).min(cu-1),y*scale)}else{label(x*scale,(y*scale+scale).min(cu-1))};
                    let expected=if enabled && label(px*scale,py*scale)!=next {(combined(px,py)+2*combined(x,y)+combined(nx,ny))/4}else{combined(x,y)};
                    assert_eq!(frame.decoded.picture.planes[c].samples()[(by+y)*side+bx+x],expected as u16,"{name} c{c} {x},{y}");
                }}
                }}
            }
            let size=case["size"].as_u64().unwrap()as usize;
            for y in (0..size).step_by(8) {for x in (0..size).step_by(8) {
                let motion=frame.decoded.picture.prediction_motion_at([x as u32,y as u32]).unwrap();
                let second=if vertical {x%cu>=cu/2}else{y%cu>=cu/2};
                let vector=motion[0].unwrap();
                assert!(motion[1].is_none());
                assert_eq!(vector.reference,u8::from(second),"{name} motion {x},{y}");
                assert_eq!(vector.mv,[0,0],"{name} vector {x},{y}");
            }}
        }
        assert_eq!(count,6,"{name}");
        reader.rewind();
        }
    }
    }
}

#[test]
fn fractional_dbbp_matches_convolution_oracle_and_retains_both_pu_motions() {
    check_motion_dbbp("dbbp-fractional",16);
    check_motion_dbbp("dbbp-merge",8);
    check_motion_dbbp("dbbp-iv",10);
    check_motion_dbbp("dbbp-weighted",16);
    check_motion_dbbp("dbbp-residual",32);
    check_motion_dbbp("dbbp-ac-vertical",32);
    check_motion_dbbp("dbbp-ac-horizontal",32);
}
fn check_motion_dbbp(prefix:&str,cases:usize) {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join(format!("{prefix}.json"))).unwrap()).unwrap();
    assert!(manifest["different_from_zero_motion"].as_u64().unwrap()>1000);
    if prefix=="dbbp-residual" || prefix.starts_with("dbbp-ac-") {assert!(manifest["residual_order_witnesses"].as_u64().unwrap()>0);}
    if prefix=="dbbp-weighted" {assert!(manifest["early_rounding_witnesses"].as_u64().unwrap()>0);}
    let golden=std::fs::read(root.join(format!("{prefix}.y16"))).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(),cases);
    for case in manifest["cases"].as_array().unwrap() {
        let name=case["file"].as_str().unwrap();
        let mut reader=LayeredMp4VideoReader::open(Cursor::new(std::fs::read(root.join(name)).unwrap()),Limits::default(),1,0,64<<20).unwrap();
        for _ in 0..2 {
            let mut count=0;
            while let Some(frame)=reader.read_frame().unwrap() {
                count+=1;
                if frame.decoded.layer_id!=5 || frame.decoded.poc!=1 {continue;}
                let mut at=case["offset"].as_u64().unwrap()as usize*2;
                for (c,plane) in frame.decoded.picture.planes.iter().enumerate() {
                    for (index,&actual) in plane.samples().iter().enumerate() {
                        let expected=u16::from_le_bytes([golden[at],golden[at+1]]);at+=2;
                        assert_eq!(actual,expected,"{name}: c{c} index{index}");
                    }
                }
                for y in (0..32).step_by(8) {for x in (0..32).step_by(8) {
                    let second=if case["vertical"].as_bool().unwrap(){x>=16}else{y>=16};
                    let expected=if case["motion_grid"].is_array(){&case["motion_grid"][(y/8*4+x/8)as usize]}else{&case["mvs"][usize::from(second)]};
                    let motion=frame.decoded.picture.prediction_motion_at([x,y]).unwrap();
                    let vector=motion[0].unwrap();
                    assert!(motion[1].is_none());
                    assert_eq!(vector.reference,case["references"][usize::from(second)].as_u64().unwrap()as u8,"{name} reference");
                    assert_eq!(vector.mv,[expected[0].as_i64().unwrap(),expected[1].as_i64().unwrap()],"{name} motion {x},{y}");
                }}
            }
            assert_eq!(count,6,"{name}");
            reader.rewind();
        }
    }
}

#[test]
fn b_dbbp_reduces_bi_merge_to_l0_and_preserves_both_lists_when_disabled() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("dbbp-b.json")).unwrap()).unwrap();
    assert!(manifest["l1_reduction_witnesses"].as_u64().unwrap()>1000);
    let golden=std::fs::read(root.join("dbbp-b.y16")).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(),34);
    for case in manifest["cases"].as_array().unwrap() {
        let name=case["file"].as_str().unwrap();
        let mut reader=LayeredMp4VideoReader::open(Cursor::new(std::fs::read(root.join(name)).unwrap()),Limits::default(),1,0,64<<20).unwrap();
        for _ in 0..2 {
            let mut count=0;let mut target=0;
            while let Some(frame)=reader.read_frame().unwrap_or_else(|e|panic!("{name}: {e}")) {
                count+=1;
                if frame.decoded.layer_id!=5 || frame.decoded.poc!=1 {continue;}
                target+=1;
                let mut at=case["offset"].as_u64().unwrap()as usize*2;
                for (c,plane)in frame.decoded.picture.planes.iter().enumerate() {
                    for (index,&actual)in plane.samples().iter().enumerate() {
                        let expected=u16::from_le_bytes([golden[at],golden[at+1]]);at+=2;
                        assert_eq!(actual,expected,"{name} c{c} index{index}");
                    }
                }
                let size=case["size"].as_u64().unwrap()as u32;
                for y in (0..size).step_by(8) {for x in (0..size).step_by(8) {
                    let second=if case["vertical"].as_bool().unwrap(){x%32>=16}else{y%32>=16};
                    let motion=frame.decoded.picture.prediction_motion_at([x,y]).unwrap();
                    assert_eq!(motion[0].unwrap().reference,0,"{name} L0 {x},{y}");
                    assert_eq!(motion[0].unwrap().mv,[0,0]);
                    let bi=if second {!case["enabled"].as_bool().unwrap()}else{case["explicit_bi"].as_bool().unwrap()};
                    assert_eq!(motion[1].is_some(),bi,"{name} L1 {x},{y}");
                    if let Some(vector)=motion[1] {assert_eq!(vector.reference,0);assert_eq!(vector.mv,[0,0]);}
                }}
            }
            assert_eq!((count,target),(9,1),"{name}");
            reader.rewind();
        }
    }
}

#[test]
fn dbbp_rejects_explicit_bi_syntax_with_specific_conformance_error() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("dbbp-b.json")).unwrap()).unwrap();
    let cases=manifest["refusal_cases"].as_array().unwrap();assert_eq!(cases.len(),2);
    for case in cases {
        let name=case["file"].as_str().unwrap();
        let mut reader=LayeredMp4VideoReader::open(Cursor::new(std::fs::read(root.join(name)).unwrap()),Limits::default(),1,0,64<<20).unwrap();
        let error=loop {match reader.read_frame() {
            Ok(Some(_))=>{},Ok(None)=>panic!("{name}: explicit BI must be refused with DBBP"),Err(error)=>break error,
        }};
        assert!(error.to_string().contains(case["reason"].as_str().unwrap()),"{name}: {error}");
    }
}

#[test]
fn b_dbbp_iv_uses_default_fractional_l0_instead_of_bi_subblocks() {
    check_b_dbbp_iv("dbbp-b-iv",76,false);
}
#[test]
fn b_dbbp_iv_retains_only_l1_and_uses_l1_weights() {
    check_b_dbbp_iv("dbbp-b-iv-l1",76,true);
}
fn check_b_dbbp_iv(prefix:&str,cases:usize,l1_only:bool) {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join(format!("{prefix}.json"))).unwrap()).unwrap();
    assert!(manifest[if l1_only {"l1_retention_witnesses"}else{"l1_reduction_witnesses"}].as_u64().unwrap()>1000);
    assert!(manifest["subblock_witnesses"].as_u64().unwrap()>1000);
    let golden=std::fs::read(root.join(format!("{prefix}.y16"))).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(),cases);
    for case in manifest["cases"].as_array().unwrap() {
        let name=case["file"].as_str().unwrap();
        let mut reader=LayeredMp4VideoReader::open(Cursor::new(std::fs::read(root.join(name)).unwrap()),Limits::default(),1,0,64<<20).unwrap();
        for _ in 0..2 {
            let mut count=0;let mut target=0;
            while let Some(frame)=reader.read_frame().unwrap_or_else(|e|panic!("{name}: {e}")) {
                count+=1;
                if frame.decoded.layer_id!=5 || frame.decoded.poc!=1 {continue;}
                target+=1;
                let size=case["size"].as_u64().unwrap()as u32;
                for y in (0..size).step_by(32) {for x in (0..size).step_by(32) {
                    let motion=frame.decoded.picture.prediction_motion_at([x,y]).unwrap();
                    assert_eq!(motion[case["first_list"].as_u64().unwrap()as usize].unwrap().mv,[0,0],"{name} PU0 AMVP cancellation {x},{y}");
                }}
                let mut at=case["offset"].as_u64().unwrap()as usize*2;
                for (c,plane)in frame.decoded.picture.planes.iter().enumerate() {
                    for (index,&actual)in plane.samples().iter().enumerate() {
                        let expected=u16::from_le_bytes([golden[at],golden[at+1]]);at+=2;
                        assert_eq!(actual,expected,"{name} c{c} index{index}");
                    }
                }
                let size=case["size"].as_u64().unwrap()as u32;
                for y in (0..size).step_by(8) {for x in (0..size).step_by(8) {
                    let expected=&case["motion_grid"][(y/8*(size/8)+x/8)as usize];
                    let motion=frame.decoded.picture.prediction_motion_at([x,y]).unwrap();
                    for list in 0..2 {
                        let active=!expected[list].is_null() && (l1_only || list==0 || !case["enabled"].as_bool().unwrap());
                        assert_eq!(motion[list].is_some(),active,"{name} list{list} {x},{y}");
                        if let Some(vector)=motion[list] {
                            assert_eq!(vector.reference,0);
                            assert_eq!(vector.mv,[expected[list][0].as_i64().unwrap(),expected[list][1].as_i64().unwrap()],"{name} list{list} {x},{y}");
                        }
                    }
                }}
            }
            assert_eq!((count,target),(9,1),"{name}");
            reader.rewind();
        }
    }
}

#[test]
fn dbbp_samples_depth_at_refined_signed_disparity_with_picture_padding() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("dbbp-disparity.json")).unwrap()).unwrap();
    assert!(manifest["different_from_zero_disparity"].as_u64().unwrap()>1000);
    assert!(manifest["padded_blocks"].as_u64().unwrap()>0);
    assert!(manifest["negative_offsets"].as_u64().unwrap()>0);
    let golden=std::fs::read(root.join("dbbp-disparity.y16")).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(),160);
    for case in manifest["cases"].as_array().unwrap() {
        let name=case["file"].as_str().unwrap();
        let size=case["size"].as_u64().unwrap()as u32;let cu=case["cu"].as_u64().unwrap()as u32;
        let vertical=case["vertical"].as_bool().unwrap();
        let mut reader=LayeredMp4VideoReader::open(Cursor::new(std::fs::read(root.join(name)).unwrap()),Limits::default(),1,0,64<<20).unwrap();
        for _ in 0..2 {
            let mut count=0;let mut target=0;
            while let Some(frame)=reader.read_frame().unwrap_or_else(|e|panic!("{name}: {e}")) {
                count+=1;
                if frame.decoded.layer_id!=5 || frame.decoded.poc!=1 {continue;}
                target+=1;
                let mut at=case["offset"].as_u64().unwrap()as usize*2;
                for (c,plane)in frame.decoded.picture.planes.iter().enumerate() {
                    for (index,&actual)in plane.samples().iter().enumerate() {
                        let expected=u16::from_le_bytes([golden[at],golden[at+1]]);at+=2;
                        assert_eq!(actual,expected,"{name} c{c} index{index}");
                    }
                }
                for y in (0..size).step_by(8) {for x in (0..size).step_by(8) {
                    let motion=frame.decoded.picture.prediction_motion_at([x,y]).unwrap();
                    let second=if vertical{x%cu>=cu/2}else{y%cu>=cu/2};
                    let vector=motion[0].unwrap();
                    assert_eq!(vector.reference,u8::from(second),"{name} reference {x},{y}");
                    assert_eq!(vector.mv,[0,0],"{name} refinement must not replace explicit motion {x},{y}");
                    assert!(motion[1].is_none());
                }}
            }
            assert_eq!((count,target),(6,1),"{name}");
            reader.rewind();
        }
    }
}

#[test]
fn dbbp_spatial_base_disparity_moves_depth_in_both_axes_across_ctu_rows() {
    check_dbbp_base("dbbp-base",72,false);
}
#[test]
fn dbbp_refines_nonzero_base_before_replacing_the_contour_vector() {
    check_dbbp_base("dbbp-base-refined",216,true);
}
fn check_dbbp_base(prefix:&str,cases:usize,refined:bool) {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join(format!("{prefix}.json"))).unwrap()).unwrap();
    if refined {
        assert!(manifest["shifted_refinement_witnesses"].as_u64().unwrap()>1000);
        assert!(manifest["vertical_replacement_witnesses"].as_u64().unwrap()>1000);
    }else {
        assert!(manifest["different_from_zero_base"].as_u64().unwrap()>1000);
        assert!(manifest["padded_contours"].as_u64().unwrap()>0);
    }
    let golden=std::fs::read(root.join(format!("{prefix}.y16"))).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(),cases);
    for case in manifest["cases"].as_array().unwrap() {
        let name=case["file"].as_str().unwrap();let size=case["size"].as_u64().unwrap()as u32;
        let vertical=case["vertical"].as_bool().unwrap();
        let expected_motion=[case["motion"][0].as_i64().unwrap(),case["motion"][1].as_i64().unwrap()];
        let mut reader=LayeredMp4VideoReader::open(Cursor::new(std::fs::read(root.join(name)).unwrap()),Limits::default(),1,0,64<<20).unwrap();
        for _ in 0..2 {
            let mut count=0;let mut target=0;
            while let Some(frame)=reader.read_frame().unwrap_or_else(|e|panic!("{name}: {e}")) {
                count+=1;if frame.decoded.layer_id!=5 || frame.decoded.poc!=1 {continue;}target+=1;
                // Inspect motion before pixels so authored MVD/MVP mistakes
                // cannot be mistaken for a contour or interpolation defect.
                for y in (0..size).step_by(8) {for x in (0..size).step_by(8) {
                    let motion=frame.decoded.picture.prediction_motion_at([x,y]).unwrap();
                    let second=if vertical{x%32>=16}else{y%32>=16};let vector=motion[0].unwrap();
                    assert_eq!(vector.reference,u8::from(second),"{name} reference {x},{y}");
                    assert_eq!(vector.mv,if second{expected_motion}else{[0,0]},"{name} motion {x},{y}");
                    assert!(motion[1].is_none());
                }}
                let mut at=case["offset"].as_u64().unwrap()as usize*2;
                for (c,plane)in frame.decoded.picture.planes.iter().enumerate() {for (index,&actual)in plane.samples().iter().enumerate() {
                    let expected=u16::from_le_bytes([golden[at],golden[at+1]]);at+=2;
                    assert_eq!(actual,expected,"{name} c{c} index{index}");
                }}
            }
            assert_eq!((count,target),(6,1),"{name}");reader.rewind();
        }
    }
}
