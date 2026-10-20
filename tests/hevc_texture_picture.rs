use fvid::{
    codec::{
        hevc_decoder::{LayeredAccessUnit, LayeredParameters},
        hevc_inter_layer::PictureStatus,
        hevc_picture::{Picture, select_texture_picture},
        hevc_poc::LayeredPocState,
        hevc_reference_list::PictureIdentity,
    },
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::LayeredMp4VideoReader,
};
use std::{io::Cursor, sync::Arc};
fn bytes(label: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/hevc-wedge/in-component-{label}-playback.mp4"
        )),
    )
    .unwrap()
}
fn plan(label: &str) -> LayeredAccessUnit {
    let mut movie = Mp4Reader::open(Cursor::new(bytes(label)), Limits::default()).unwrap();
    let params =
        LayeredParameters::from_configuration(&movie.tracks()[0].configuration, 64 << 20).unwrap();
    let mut packet = Vec::new();
    movie.read_packet(0, 0, &mut packet).unwrap();
    LayeredAccessUnit::parse(
        &params,
        &LayeredPocState::default(),
        &packet,
        4,
        1,
        64 << 20,
    )
    .unwrap()
}
fn texture() -> (PictureStatus, Arc<Picture>) {
    let mut reader =
        LayeredMp4VideoReader::open(Cursor::new(bytes("off")), Limits::default(), 1, 0, 64 << 20)
            .unwrap();
    let frame = reader.read_frame().unwrap().unwrap();
    assert_eq!(frame.decoded.layer_id, 0);
    (
        PictureStatus {
            identity: PictureIdentity {
                layer_id: 0,
                poc: frame.decoded.poc,
            },
            access_unit: frame.decoded.access_unit,
            nal: frame.decoded.nal,
            discardable: false,
        },
        frame.decoded.picture.clone(),
    )
}
#[test]
fn texture_selection_shares_only_completed_current_au_picture() {
    let planned = plan("on");
    let depth = planned
        .pictures()
        .iter()
        .find(|p| p.binding.sps.layer_id == 3)
        .unwrap();
    let context = depth.binding.context();
    let header = &depth.slices[0];
    let (status, picture) = texture();
    let selected =
        select_texture_picture(&context, header, depth.poc, 0, &[(status, picture.clone())])
            .unwrap()
            .unwrap();
    assert!(Arc::ptr_eq(&selected, &picture));
    assert_eq!(
        select_texture_picture(&context, header, depth.poc, 0, &[])
            .err()
            .unwrap()
            .to_string(),
        "HEVC TexturePic is not completed in current access unit"
    );
    for (au, poc) in [(1, 0), (0, 1)] {
        let mut wrong = status;
        wrong.access_unit = au;
        wrong.identity.poc = poc;
        assert_eq!(
            select_texture_picture(&context, header, depth.poc, 0, &[(wrong, picture.clone())])
                .err()
                .unwrap()
                .to_string(),
            "HEVC TexturePic access unit or POC disagrees"
        );
    }
    assert_eq!(
        select_texture_picture(
            &context,
            header,
            depth.poc,
            0,
            &[(status, picture.clone()), (status, picture.clone())]
        )
        .err()
        .unwrap()
        .to_string(),
        "HEVC TexturePic repeats a layer identity"
    );
    let mut bad = status;
    bad.nal.layer_id = 3;
    assert!(select_texture_picture(&context, header, depth.poc, 0, &[(bad, picture)]).is_err());
    let disabled = plan("off");
    let depth = disabled
        .pictures()
        .iter()
        .find(|p| p.binding.sps.layer_id == 3)
        .unwrap();
    assert!(
        select_texture_picture(
            &depth.binding.context(),
            &depth.slices[0],
            depth.poc,
            0,
            &[]
        )
        .unwrap()
        .is_none()
    );
}
#[test]
fn incomplete_texture_samples_are_rejected_before_depth_reconstruction() {
    let planned = plan("on");
    let depth = planned
        .pictures()
        .iter()
        .find(|p| p.binding.sps.layer_id == 3)
        .unwrap();
    let (status, mut picture) = texture();
    Arc::get_mut(&mut picture).unwrap().planes[0] =
        fvid::codec::hevc_plane::Plane::new(64, 64, 8, 1 << 20).unwrap();
    assert_eq!(
        select_texture_picture(
            &depth.binding.context(),
            &depth.slices[0],
            depth.poc,
            0,
            &[(status, picture)]
        )
        .err()
        .unwrap()
        .to_string(),
        "HEVC TexturePic luma is incomplete or does not cover depth picture"
    );
}
#[test]
fn active_contour_decode_requires_explicit_texture_handoff() {
    let planned = plan("on");
    let depth = planned
        .pictures()
        .iter()
        .find(|p| p.binding.sps.layer_id == 3)
        .unwrap();
    let lists = vec![[Vec::new(), Vec::new()]; depth.slices.len()];
    assert_eq!(
        fvid::codec::hevc_picture::decode_layered(
            &depth.binding.context(),
            &depth.slices,
            depth.poc,
            &lists,
            64 << 20
        )
        .err()
        .unwrap()
        .to_string(),
        "HEVC contour prediction requires current-AU TexturePic"
    );
}
