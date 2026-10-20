//! Coded source acceptance plus depth-domain resampling; no claim that this
//! standalone component witness is a coded inter-layer resampling movie.
use fvid::{codec::{hevc_decoder::{LayeredAccessUnit,LayeredParameters},
    hevc_poc::LayeredPocState,hevc_pps::ReferenceLocation,
    hevc_resample::{MotionResampling,MotionSource,Resampling}},
    container::mp4::{Limits,Mp4Reader},playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn coded_depth_motion_resamples_in_integer_units_without_fraction_or_quarter_clipping() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let bytes=std::fs::read(root.join("depth-mvd-resampling.mp4")).unwrap();
    let expected:Vec<_>=std::fs::read(root.join("depth-mvd-resampling.y16")).unwrap()
        .chunks_exact(2).map(|v|u16::from_le_bytes([v[0],v[1]])).collect();
    let mut input=Mp4Reader::open(std::io::Cursor::new(bytes.clone()),Limits::default()).unwrap();
    let params=LayeredParameters::from_configuration(&input.tracks()[0].configuration,64<<20).unwrap();
    let mut packet=Vec::new();input.read_packet(0,0,&mut packet).unwrap();
    let plan=LayeredAccessUnit::parse(&params,&LayeredPocState::default(),&packet,4,1,64<<20).unwrap();
    let reference=plan.pictures().iter().find(|p|p.binding.sps.layer_id==3).unwrap().binding.sps.sps.clone();
    assert!(reference.depth_layer);
    let location=ReferenceLocation{layer_id:3,scaled_offsets:None,region_offsets:None,phases:None};
    let mut reader=LayeredMp4VideoReader::open(std::io::Cursor::new(bytes),Limits::default(),1,0,64<<20).unwrap();
    for _ in 0..2 {
        let mut count=0;
        while let Some(frame)=reader.read_frame().unwrap() {
            if frame.decoded.layer_id!=3{continue;}
            let picture=&frame.decoded.picture;
            let wanted=&expected[count*1024..(count+1)*1024];
            assert_eq!(picture.planes[0].samples().iter().zip(wanted).enumerate().find(|(_, (a,b))|a!=b),None);
            if count>0 {
                let grid:Vec<_>=(0..2).flat_map(|y|(0..2).map(move|x|picture.prediction_motion_at([x*16,y*16]).unwrap())).collect();
                let snapshot=grid.clone();
                assert_eq!(grid[0][0].unwrap().mv,if count==1{[0,4]}else{[131068,-131072]});
                for dimensions in [[32,32],[48,48],[64,64],[48,64]] {
                    let mut current=reference.clone();current.dimensions=dimensions;
                    let operation=MotionResampling::new(&current,&reference,&location).unwrap();
                    let actual=operation.motion(MotionSource::Grid(&grid),1<<20).unwrap();
                    let expected_motion=if count==1{[0,if dimensions[1]==64{8}else{4}]}else{[131068,-131072]};
                    let mut active=0;
                    for motion in &actual {for vector in motion.iter().flatten() {
                        active+=1;assert_eq!(vector.mv,expected_motion,"POC{count} scaled to{dimensions:?}");
                    }}
                    assert!(active>0);
                    assert_eq!(Resampling::new(&current,&reference,&location).unwrap()
                        .motion(MotionSource::Grid(&grid),1<<20).unwrap(),actual);
                    assert_eq!(grid,snapshot);
                    assert!(operation.motion(MotionSource::Grid(&grid),0).is_err());
                }
            }
            count+=1;
        }
        assert_eq!(count,3);reader.rewind();
    }
}
