use fvid::{
    codec::{
        hevc_decoder::{LayeredAccessUnit, LayeredDpb, LayeredParameters},
        hevc_inter_layer::PictureStatus,
        hevc_motion::inter_view_residual::{BoundSources, CurrentRps},
        hevc_picture::ViewComponentPool,
        hevc_reference_list::PictureIdentity,
    },
    container::mp4::Mp4Reader,
};
use std::{io::Cursor, sync::Arc};

#[test]
fn two_texture_views_bind_by_identity_and_reject_duplicate_or_wrong_au_rps() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let mut movie = Mp4Reader::open(
        Cursor::new(std::fs::read(root.join("inter-view-temporal-scaling-on-equal.mp4")).unwrap()),
        Default::default(),
    )
    .unwrap();
    let parameters =
        LayeredParameters::from_configuration(&movie.tracks()[0].configuration, 64 << 20).unwrap();
    let mut dpb = LayeredDpb::default();
    let mut packet = Vec::new();
    movie.read_packet(0, 0, &mut packet).unwrap();
    let plan = LayeredAccessUnit::parse(&parameters, dpb.poc(), &packet, 4, 1, 64 << 20).unwrap();
    let bindings: Vec<_> = plan.pictures().iter().map(|p| p.binding.clone()).collect();
    let header = plan
        .pictures()
        .iter()
        .find(|p| p.binding.sps.layer_id == 5)
        .unwrap()
        .slices[0]
        .clone();
    let temporal: Vec<_> = dpb.references().cloned().collect();
    let mut snapshots: Vec<_> = plan
        .pictures()
        .iter()
        .filter(|p| p.binding.sps.layer_id < 5)
        .map(|p| CurrentRps::bind(p, 0, dpb.next_access_unit(), &temporal, 64 << 20).unwrap())
        .collect();
    let stale: Vec<_> = plan
        .pictures()
        .iter()
        .filter(|p| p.binding.sps.layer_id < 5)
        .map(|p| CurrentRps::bind(p, 0, dpb.next_access_unit() + 1, &temporal, 64 << 20).unwrap())
        .collect();
    let (_, frames, _) = dpb.decode(plan, 64 << 20).unwrap();
    let status = |p: &fvid::codec::hevc_decoder::LayeredDecoded| PictureStatus {
        identity: PictureIdentity {
            layer_id: p.layer_id,
            poc: p.poc,
        },
        access_unit: p.access_unit,
        nal: p.nal,
        discardable: false,
    };
    let target = frames.iter().find(|p| p.layer_id == 5).unwrap();
    let completed: Vec<_> = frames
        .iter()
        .filter(|p| p.layer_id < 5)
        .map(|p| (status(p), p.picture.clone()))
        .collect();
    let context = target.binding.context();
    let components = ViewComponentPool::bind(
        &context,
        &header,
        status(target),
        &bindings,
        false,
        &completed,
        65536,
    )
    .unwrap();
    assert_eq!(components.len(), 2);
    snapshots.reverse();
    let sources = BoundSources::bind(
        &components,
        &context,
        &header,
        status(target),
        &snapshots,
        &temporal,
    )
    .unwrap();
    for view in components.views() {
        assert!(Arc::ptr_eq(
            &sources.current_view(view).unwrap(),
            components.get(view).unwrap()
        ));
        assert!(sources.temporal_view(view, target.poc).unwrap().is_none());
    }
    let duplicate = [snapshots[0].clone(), snapshots[0].clone()];
    assert!(
        BoundSources::bind(
            &components,
            &context,
            &header,
            status(target),
            &duplicate,
            &temporal
        )
        .is_err()
    );
    assert!(
        BoundSources::bind(
            &components,
            &context,
            &header,
            status(target),
            &stale,
            &temporal
        )
        .is_err()
    );
}
