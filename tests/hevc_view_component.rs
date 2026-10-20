use fvid::{
    codec::{
        hevc_decoder::{LayeredAccessUnit, LayeredDecoded, LayeredParameters},
        hevc_inter_layer::PictureStatus,
        hevc_picture::{Picture, ViewComponentPool, select_current_view_component},
        hevc_poc::LayeredPocState,
        hevc_reference_list::PictureIdentity,
    },
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::LayeredMp4VideoReader,
};
use std::{io::Cursor, sync::Arc};
fn load(name: &str) -> (LayeredAccessUnit, Vec<LayeredDecoded>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "tests/fixtures/playback-errors/hevc-wedge/{name}.mp4"
    ));
    let data = std::fs::read(path).unwrap();
    let mut input = Mp4Reader::open(Cursor::new(data.clone()), Limits::default()).unwrap();
    let parameters =
        LayeredParameters::from_configuration(&input.tracks()[0].configuration, 64 << 20).unwrap();
    let mut packet = Vec::new();
    input.read_packet(0, 0, &mut packet).unwrap();
    let plan = LayeredAccessUnit::parse(
        &parameters,
        &LayeredPocState::default(),
        &packet,
        4,
        1,
        64 << 20,
    )
    .unwrap();
    let mut reader =
        LayeredMp4VideoReader::open(Cursor::new(data), Limits::default(), 1, 0, 64 << 20).unwrap();
    let mut pictures = Vec::new();
    while let Some(frame) = reader.read_frame().unwrap() {
        if frame.decoded.poc == 0 {
            pictures.push(frame.decoded);
        }
    }
    (plan, pictures)
}
fn modified_picture(name: &str, layer: u8, change: impl FnOnce(&mut Picture)) -> Arc<Picture> {
    let (_, mut decoded) = load(name);
    let index = decoded.iter().position(|p| p.layer_id == layer).unwrap();
    let mut selected = decoded.swap_remove(index);
    change(
        Arc::get_mut(&mut selected.picture)
            .expect("decoded source has one owner after reader drop"),
    );
    selected.picture
}
fn status(p: &LayeredDecoded) -> PictureStatus {
    PictureStatus {
        identity: PictureIdentity {
            layer_id: p.layer_id,
            poc: p.poc,
        },
        access_unit: p.access_unit,
        nal: p.nal,
        discardable: false,
    }
}
fn completed(pictures: &[LayeredDecoded], before: u8) -> Vec<(PictureStatus, Arc<Picture>)> {
    pictures
        .iter()
        .filter(|p| p.layer_id < before)
        .map(|p| (status(p), p.picture.clone()))
        .collect()
}

#[test]
fn component_pool_keeps_current_au_sources_outside_active_reference_lists() {
    for (name, current_id, depth, expected) in [
        ("disparity-depth-source", 8, true, vec![(0, 3), (1, 5)]),
        (
            "inter-view-temporal-scaling-on-equal",
            5,
            false,
            vec![(0, 0), (1, 3)],
        ),
    ] {
        let (plan, pictures) = load(name);
        let current = pictures.iter().find(|p| p.layer_id == current_id).unwrap();
        let header = &plan
            .pictures()
            .iter()
            .find(|p| p.binding.sps.layer_id == current_id)
            .unwrap()
            .slices[0];
        assert_eq!(header.references, [0, 0], "I-picture RPLs are empty");
        let context = current.binding.context();
        let done = completed(&pictures, current_id);
        let bindings: Vec<_> = plan.pictures().iter().map(|p| p.binding.clone()).collect();
        let pool = ViewComponentPool::bind(
            &context,
            header,
            status(current),
            &bindings,
            depth,
            &done,
            4096,
        )
        .unwrap();
        assert_eq!(pool.len(), expected.len());
        assert!(!pool.is_empty());
        assert_eq!(pool.depth(), depth);
        assert!(pool.metadata_bytes() <= 4096);
        for (view, id) in expected {
            assert_eq!(pool.layer(view), Some(id));
            let original = &pictures.iter().find(|p| p.layer_id == id).unwrap().picture;
            assert!(Arc::ptr_eq(pool.get(view).unwrap(), original));
        }
        assert!(pool.get(65535).is_none());
        assert_eq!(pool.layer(65535), None);
        pool.validate_for(&context, header, status(current))
            .unwrap();
        let mut stale = status(current);
        stale.access_unit += 1;
        assert!(pool.validate_for(&context, header, stale).is_err());
        let mut changed = current.binding.as_ref().clone();
        changed.pps.pps.parallel_merge_log2 += 1;
        assert!(
            pool.validate_for(&changed.context(), header, status(current))
                .is_err()
        );
        let planned = plan
            .pictures()
            .iter()
            .find(|p| p.binding.sps.layer_id == current_id)
            .unwrap();
        let lists = vec![[Vec::new(), Vec::new()]; planned.slices.len()];
        let decoded = fvid::codec::hevc_picture::decode_layered_with_components(
            &context,
            &planned.slices,
            current.poc,
            &lists,
            64 << 20,
            None,
            Some((&pool, status(current))),
        )
        .unwrap();
        for component in 0..3 {
            assert_eq!(
                decoded.planes[component].samples(),
                current.picture.planes[component].samples()
            );
        }
        assert!(
            fvid::codec::hevc_picture::decode_layered_with_components(
                &context,
                &planned.slices,
                current.poc,
                &lists,
                64 << 20,
                None,
                Some((&pool, stale))
            )
            .is_err()
        );
        assert!(
            ViewComponentPool::bind(
                &context,
                header,
                status(current),
                &bindings,
                depth,
                &done,
                pool.metadata_bytes() - 1
            )
            .is_err()
        );
    }
    assert!(ViewComponentPool::required_metadata_bytes(64).is_none());
    assert!(ViewComponentPool::required_metadata_bytes(usize::MAX).is_none());
}

#[test]
fn component_pool_rejects_stale_duplicate_missing_or_forged_source_bindings() {
    let (plan, pictures) = load("disparity-depth-source");
    let current = pictures.iter().find(|p| p.layer_id == 8).unwrap();
    let header = &plan
        .pictures()
        .iter()
        .find(|p| p.binding.sps.layer_id == 8)
        .unwrap()
        .slices[0];
    let context = current.binding.context();
    let now = status(current);
    let done = completed(&pictures, 8);
    let bindings: Vec<_> = plan.pictures().iter().map(|p| p.binding.clone()).collect();
    let bind = |sources: &[(PictureStatus, Arc<Picture>)],
                snapshots: &[Arc<fvid::codec::hevc_decoder::LayeredBinding>]| {
        ViewComponentPool::bind(&context, header, now, snapshots, true, sources, 4096)
    };
    assert!(bind(&done, &[]).is_err());
    let mut snapshots = bindings.clone();
    snapshots.push(
        bindings
            .iter()
            .find(|b| b.sps.layer_id == 3)
            .unwrap()
            .clone(),
    );
    assert!(bind(&done, &snapshots).is_err());
    let mut wrong = done.clone();
    wrong[1].0.access_unit += 1;
    assert!(bind(&wrong, &bindings).is_err());
    wrong = done.clone();
    wrong[1].0.identity.poc += 1;
    assert!(bind(&wrong, &bindings).is_err());
    wrong = done.clone();
    wrong.push(wrong[1].clone());
    assert!(bind(&wrong, &bindings).is_err());
    wrong = done.clone();
    wrong[1].0.nal.unit_type = 1;
    let error = bind(&wrong, &bindings).err().unwrap().to_string();
    assert!(
        error.contains("NAL disagrees with decoded picture"),
        "{error}"
    );
    let empty = ViewComponentPool::bind(&context, header, now, &bindings, true, &[], 4096).unwrap();
    assert!(empty.is_empty());
    assert!(empty.get(0).is_none());
}

#[test]
fn component_pool_indexes_only_canonical_non_auxiliary_components() {
    let (plan, pictures) = load("disparity-depth-source");
    let current = pictures.iter().find(|p| p.layer_id == 8).unwrap();
    let header = &plan
        .pictures()
        .iter()
        .find(|p| p.binding.sps.layer_id == 8)
        .unwrap()
        .slices[0];
    let mut vps = current.binding.vps.as_ref().clone();
    // Source layer5 is auxiliary; it must not replace or invalidate ordinary
    // depth sources merely because it is also a completed monochrome picture.
    vps.extension.as_mut().unwrap().graph.scalability_ids[2][2] = 1;
    let vps = Arc::new(vps);
    let bindings: Vec<_> = plan
        .pictures()
        .iter()
        .map(|p| {
            let mut b = p.binding.as_ref().clone();
            b.vps = vps.clone();
            Arc::new(b)
        })
        .collect();
    let binding = bindings.iter().find(|b| b.sps.layer_id == 8).unwrap();
    let pool = ViewComponentPool::bind(
        &binding.context(),
        header,
        status(current),
        &bindings,
        true,
        &completed(&pictures, 8),
        4096,
    )
    .unwrap();
    assert_eq!(pool.len(), 1);
    assert_eq!(pool.layer(0), Some(3));
    assert!(pool.get(1).is_none());
    assert!(Arc::ptr_eq(
        pool.get(0).unwrap(),
        &pictures.iter().find(|p| p.layer_id == 3).unwrap().picture
    ));
}
#[test]
fn explicit_view_order_and_role_select_shared_current_au_source() {
    for (name, current_id, requested_depth, expected_layers) in [
        ("disparity-depth-source", 8, true, vec![(0, 3), (1, 5)]),
        (
            "inter-view-temporal-scaling-on-equal",
            5,
            false,
            vec![(0, 0), (1, 3)],
        ),
    ] {
        let (plan, pictures) = load(name);
        let current = pictures.iter().find(|p| p.layer_id == current_id).unwrap();
        let header = &plan
            .pictures()
            .iter()
            .find(|p| p.binding.sps.layer_id == current_id)
            .unwrap()
            .slices[0];
        let context = current.binding.context();
        let done = completed(&pictures, current_id);
        let graph = &context.vps.extension.as_ref().unwrap().graph;
        for (view, layer) in expected_layers {
            assert_eq!(
                graph
                    .view_component_for_index(view, requested_depth)
                    .unwrap(),
                Some(layer)
            );
            let source = pictures.iter().find(|p| p.layer_id == layer).unwrap();
            let selected = select_current_view_component(
                &context,
                header,
                status(current),
                &source.binding,
                view,
                requested_depth,
                &done,
            )
            .unwrap();
            assert!(Arc::ptr_eq(&selected, &source.picture));
            // Sharing keeps the decoded source unchanged and available to other tools.
            assert_eq!(
                selected.planes[0].samples(),
                source.picture.planes[0].samples()
            );
        }
        assert!(
            graph
                .view_component_for_index(65535, requested_depth)
                .unwrap()
                .is_none()
        );
        if requested_depth {
            // View 0 texture is layer 0, its depth component is layer 3.
            assert_eq!(graph.view_component_for_index(0, false).unwrap(), Some(0));
            assert_eq!(graph.view_component_for_index(0, true).unwrap(), Some(3));
        } else {
            assert_eq!(graph.view_ids, [7, 7, 7]);
        }
    }
}
#[test]
fn stale_ambiguous_wrong_role_and_incomplete_sources_are_rejected() {
    let (plan, pictures) = load("disparity-depth-source");
    let current = pictures.iter().find(|p| p.layer_id == 8).unwrap();
    let source = pictures.iter().find(|p| p.layer_id == 3).unwrap();
    let header = &plan
        .pictures()
        .iter()
        .find(|p| p.binding.sps.layer_id == 8)
        .unwrap()
        .slices[0];
    let context = current.binding.context();
    let now = status(current);
    let source_status = status(source);
    let select = |done: &[(PictureStatus, Arc<Picture>)]| {
        select_current_view_component(&context, header, now, &source.binding, 0, true, done)
    };
    assert!(select(&[]).is_err());
    for (au, poc) in [
        (now.access_unit + 1, now.identity.poc),
        (now.access_unit, now.identity.poc + 1),
    ] {
        let mut wrong = source_status;
        wrong.access_unit = au;
        wrong.identity.poc = poc;
        assert_eq!(
            select(&[(wrong, source.picture.clone())])
                .err()
                .unwrap()
                .to_string(),
            "HEVC reference view-component access unit or POC disagrees"
        );
    }
    assert!(
        select(&[
            (source_status, source.picture.clone()),
            (source_status, source.picture.clone())
        ])
        .is_err()
    );
    let mut wrong = source_status;
    wrong.nal.layer_id = 0;
    assert!(select(&[(wrong, source.picture.clone())]).is_err());
    let done = completed(&pictures, 8);
    let mut other_snapshot = (*source.binding).clone();
    Arc::make_mut(&mut other_snapshot.vps)
        .extension
        .as_mut()
        .unwrap()
        .graph
        .view_ids[0] ^= 1;
    assert_eq!(
        select_current_view_component(&context, header, now, &other_snapshot, 0, true, &done)
            .err()
            .unwrap()
            .to_string(),
        "HEVC reference view component uses a different VPS snapshot"
    );
    let other = pictures.iter().find(|p| p.layer_id == 5).unwrap();
    assert!(
        select_current_view_component(&context, header, now, &other.binding, 0, true, &done)
            .is_err()
    );
    assert!(
        select_current_view_component(&context, header, now, &source.binding, 0, false, &done)
            .is_err()
    );
    assert!(
        select_current_view_component(&context, header, now, &source.binding, 100, true, &done)
            .is_err()
    );
    let incomplete = modified_picture("disparity-depth-source", 3, |p| {
        p.planes[0] = fvid::codec::hevc_plane::Plane::new(32, 32, 8, 1 << 20).unwrap();
    });
    assert_eq!(
        select(&[(source_status, incomplete)])
            .err()
            .unwrap()
            .to_string(),
        "HEVC reference view-component planes are incomplete or inconsistent"
    );
    let chroma = modified_picture("disparity-depth-source", 3, |p| {
        p.planes[1] = fvid::codec::hevc_plane::Plane::new(16, 16, 8, 1 << 20).unwrap();
    });
    assert!(select(&[(source_status, chroma)]).is_err());
    let wrong = modified_picture("disparity-depth-source", 3, |p| p.depth[0] = 10);
    assert!(select(&[(source_status, wrong)]).is_err());
}
#[test]
fn completed_chroma_and_actual_plane_precision_are_validated_against_bound_sps() {
    let (plan, pictures) = load("vsp-chroma-source");
    let current = pictures.iter().find(|p| p.layer_id == 5).unwrap();
    let source = &pictures[0];
    let header = &plan
        .pictures()
        .iter()
        .find(|p| p.binding.sps.layer_id == 5)
        .unwrap()
        .slices[0];
    let context = current.binding.context();
    let picture = modified_picture("vsp-chroma-source", 0, |p| {
        p.planes[2] = fvid::codec::hevc_plane::Plane::new(16, 16, 8, 1 << 20).unwrap();
    });
    assert_eq!(
        select_current_view_component(
            &context,
            header,
            status(current),
            &source.binding,
            0,
            false,
            &[(status(source), picture)]
        )
        .err()
        .unwrap()
        .to_string(),
        "HEVC reference view-component planes are incomplete or inconsistent"
    );
    let mut source_binding = (*source.binding).clone();
    source_binding.sps.sps.depth[0] = 10;
    let mut current_sps = current.binding.sps.clone();
    current_sps.sps.depth[0] = 10;
    let adjusted = fvid::codec::hevc_slice::LayeredSliceContext {
        vps: context.vps,
        sps: &current_sps,
        pps: context.pps,
    };
    let picture = modified_picture("vsp-chroma-source", 0, |p| p.depth[0] = 10);
    assert_eq!(
        select_current_view_component(
            &adjusted,
            header,
            status(current),
            &source_binding,
            0,
            false,
            &[(status(source), picture)]
        )
        .err()
        .unwrap()
        .to_string(),
        "HEVC reference view-component planes are incomplete or inconsistent"
    );
}

#[test]
fn view_component_mapping_excludes_dependency_auxiliary_and_keeps_last_match() {
    let (plan, _) = load("disparity-depth-source");
    let mut graph = plan.pictures()[0]
        .binding
        .vps
        .extension
        .as_ref()
        .unwrap()
        .graph
        .clone();
    assert_eq!(graph.layer_ids, [0, 3, 5, 8]);
    graph.scalability_ids[2][1] = 0;
    assert_eq!(graph.view_component_for_index(0, true).unwrap(), Some(5));
    assert_eq!(graph.view_component_layer(3, true).unwrap(), Some(5));
    graph.scalability_ids[2][2] = 1;
    assert_eq!(graph.view_component_for_index(0, true).unwrap(), Some(3));
    graph.scalability_ids[3][1] = 0;
    graph.scalability_ids[3][3] = 1;
    assert_eq!(graph.view_component_for_index(0, true).unwrap(), Some(3));
    assert!(graph.view_component_for_index(2, true).unwrap().is_none());
}

#[test]
fn texture_refinement_binds_depth_and_camera_to_selected_reference_view() {
    use fvid::codec::hevc_motion::disparity::{derivation::Vector, samples};
    let (plan,pictures)=load("texture-depth-refinement-control");
    let current=pictures.iter().find(|p|p.layer_id==5).unwrap();
    let planned=plan.pictures().iter().find(|p|p.binding.sps.layer_id==5).unwrap();
    let header=&planned.slices[0];
    let context=current.binding.context();
    let bindings:Vec<_>=plan.pictures().iter().map(|p|p.binding.clone()).collect();
    let done=completed(&pictures,5);
    let pool=ViewComponentPool::bind(&context,header,status(current),&bindings,true,&done,4096).unwrap();
    assert_eq!(pool.layer(0),Some(3));
    assert_eq!(pool.layer(1),None,"current texture view has no depth component");
    // Camera scale 128/128 maps maximum corner depth directly to quarter-pel disparity.
    // Owned depth pattern is (7*x+11*y+37)%256, never the texture formula.
    let vector=Vector{reference_view:0,mv:[0,4]};
    let budget=samples::required_bytes([8,8]).unwrap();
    let refined=pool.refine_disparity(&context,header,status(current),vector,[0,0],8,budget).unwrap();
    assert_eq!(refined.vector,vector);
    assert_eq!(refined.refined_mv,[174,0]);
    assert!(pool.refine_disparity(&context,header,status(current),vector,[0,0],8,budget-1).is_err());
    assert!(pool.refine_disparity(&context,header,status(current),vector,[32,0],8,budget).unwrap_err().to_string().contains("CU outside picture"));
    let mut stale=status(current);stale.access_unit+=1;
    assert!(pool.refine_disparity(&context,header,stale,vector,[0,0],8,budget).is_err());
    let missing=Vector{reference_view:1,..vector};
    assert!(pool.refine_disparity(&context,header,status(current),missing,[0,0],8,budget).unwrap_err().to_string().contains("reference depth view is missing"));
    let textures=ViewComponentPool::bind(&context,header,status(current),&bindings,false,&done,4096).unwrap();
    assert!(textures.refine_disparity(&context,header,status(current),vector,[0,0],8,budget).unwrap_err().to_string().contains("requires depth component pool"));
    let mut wrong=header.clone();wrong.layered.as_mut().unwrap().camera.as_mut().unwrap().view_index=0;
    assert!(pool.refine_disparity(&context,&wrong,status(current),vector,[0,0],8,budget).unwrap_err().to_string().contains("camera belongs to another view"));
}
