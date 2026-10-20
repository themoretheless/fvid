use fvid::{
    codec::{
        hevc_cabac::SliceType,
        hevc_decoder::{LayeredAccessUnit, LayeredDpb, LayeredParameters},
        hevc_motion::{
            inter_view_residual::{CurrentRps, reference_targets},
            texture::Identity,
        },
        hevc_reference_list::{PictureIdentity, ReferenceKind},
    },
    container::mp4::Mp4Reader,
};
use std::{io::Cursor, sync::Arc};
#[test]
fn coded_intra_source_keeps_used_rps_outside_its_empty_active_lists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let bytes = std::fs::read(root.join("illumination-b-64-0-0-2-1--40-both.mp4")).unwrap();
    let mut movie = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut params =
        LayeredParameters::from_configuration(&movie.tracks()[0].configuration, 64 << 20).unwrap();
    let mut dpb = LayeredDpb::default();
    let mut packet = Vec::new();
    let mut checked = false;
    for index in 0..3 {
        movie.read_packet(0, index, &mut packet).unwrap();
        let plan = LayeredAccessUnit::parse(&params, dpb.poc(), &packet, 4, 1, 64 << 20).unwrap();
        let source = plan
            .pictures()
            .iter()
            .find(|p| p.binding.sps.layer_id == 0)
            .unwrap();
        let temporal: Vec<_> = dpb.references().cloned().collect();
        let counts: Vec<_> = temporal
            .iter()
            .map(|r| Arc::strong_count(r.picture.as_ref().unwrap()))
            .collect();
        let snapshot =
            CurrentRps::bind(source, 0, dpb.next_access_unit(), &temporal, 64 << 20).unwrap();
        assert_eq!(
            counts,
            temporal
                .iter()
                .map(|r| Arc::strong_count(r.picture.as_ref().unwrap()))
                .collect::<Vec<_>>(),
            "RPS identities must not retain decoded picture owners"
        );
        assert!(Arc::ptr_eq(snapshot.binding(), &source.binding));
        assert_eq!(
            snapshot.current().identity,
            PictureIdentity {
                layer_id: 0,
                poc: source.poc
            }
        );
        assert!(snapshot.metadata_bytes() < 64 << 20);
        assert!(CurrentRps::bind(source, 0, dpb.next_access_unit(), &temporal, 0).is_err());
        if index == 2 {
            assert_eq!(source.slices[0].slice_type, SliceType::I);
            let (_, active) = source
                .reference_lists(0, dpb.next_access_unit(), &temporal, &[], 64 << 20)
                .unwrap();
            assert!(active.iter().all(Vec::is_empty));
            let identities: Vec<_> = snapshot.entries().iter().map(|r| r.picture).collect();
            assert_eq!(
                identities,
                [
                    PictureIdentity {
                        layer_id: 0,
                        poc: 0
                    },
                    PictureIdentity {
                        layer_id: 0,
                        poc: 2
                    }
                ]
            );
            assert!(
                snapshot
                    .entries()
                    .iter()
                    .all(|r| r.kind == ReferenceKind::ShortTerm)
            );
            let l0 = [
                Identity { poc: 0, view: 1 },
                Identity { poc: 1, view: 0 },
                Identity { poc: 2, view: 1 },
            ];
            let l1 = [
                Identity { poc: 2, view: 1 },
                Identity { poc: 0, view: 1 },
                Identity { poc: 1, view: 0 },
            ];
            let targets = reference_targets([&l0, &l1], true, 1, true, &[0], |view, poc| {
                assert_eq!(view, 0);
                Ok(snapshot
                    .resolve(PictureIdentity { layer_id: 0, poc }, &temporal)?
                    .is_some())
            })
            .unwrap();
            assert!(targets.syntax_available());
            assert!(targets.available_for(0, 0).unwrap());
            assert!(targets.available_for(1, 0).unwrap());
            assert!(
                snapshot
                    .resolve(
                        PictureIdentity {
                            layer_id: 5,
                            poc: 0
                        },
                        &temporal
                    )
                    .unwrap()
                    .is_none()
            );
            let missing: Vec<_> = temporal
                .iter()
                .filter(|r| r.identity() != identities[0])
                .cloned()
                .collect();
            assert!(snapshot.resolve(identities[0], &missing).is_err());
            assert!(
                CurrentRps::bind(source, 0, dpb.next_access_unit(), &missing, 64 << 20).is_err()
            );
            let mut duplicates = temporal.clone();
            duplicates.push(temporal[0].clone());
            assert!(
                CurrentRps::bind(source, 0, dpb.next_access_unit(), &duplicates, 64 << 20).is_err()
            );
            checked = true;
        }
        let (updated, _, _) = dpb.decode(plan, 64 << 20).unwrap();
        params = updated;
        if index == 2 {
            let current: Vec<_> = dpb.references().cloned().collect();
            assert!(
                current
                    .iter()
                    .any(|r| r.identity() == snapshot.current().identity)
            );
            assert!(
                snapshot
                    .resolve(snapshot.current().identity, &current)
                    .unwrap()
                    .is_none(),
                "DPB presence must not masquerade as current-RPS membership"
            );
            for entry in snapshot.entries() {
                let resolved = snapshot.resolve(entry.picture, &current).unwrap().unwrap();
                let actual = current
                    .iter()
                    .find(|r| r.identity() == entry.picture)
                    .unwrap()
                    .picture
                    .as_ref()
                    .unwrap();
                assert!(Arc::ptr_eq(&resolved, actual));
            }
        }
    }
    assert!(checked);
}
