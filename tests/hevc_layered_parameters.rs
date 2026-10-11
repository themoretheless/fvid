//! Global parameter replacement, immutable activations and transaction rollback.
use fvid::codec::{config::HevcConfig, hevc_decoder::LayeredParameters, hevc_nal::NalHeader};
use fvid::container::mp4::Mp4Reader;
use std::{io::Cursor, path::Path, sync::Arc};

fn configuration(file: &str) -> Vec<u8> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let movie = Mp4Reader::open(
        Cursor::new(std::fs::read(root.join(file)).unwrap()),
        Default::default(),
    )
    .unwrap();
    movie.tracks()[0].configuration.clone()
}

#[test]
fn received_ids_replace_across_nal_layers_without_fallback_or_partial_activation() {
    let good = configuration("hevc-layered-reconstruct-pcm-independent.mp4");
    for kind in [33, 34] {
        let bad = configuration(&format!("hevc-layered-global-{kind}-replacement.mp4"));
        let config = HevcConfig::parse(&bad).unwrap();
        let units = &config
            .arrays
            .iter()
            .find(|a| a.nal_type == kind)
            .unwrap()
            .units;
        assert_eq!(units.len(), 2);
        assert_eq!(NalHeader::parse(units[0]).unwrap().layer_id, 0);
        assert_eq!(NalHeader::parse(units[1]).unwrap().layer_id, 3);
        let expected = "HEVC SPS NAL does not belong to the active or a reference layer".to_owned();
        let mut received = LayeredParameters::from_configuration(&bad, 32 << 20).unwrap();
        assert_eq!(
            received.activate(0, 0, 1).unwrap_err().to_string(),
            expected
        );
        assert!(received.active(0).is_none());
        // Re-receiving the base parameter restores the global ID; no physical
        // layer namespace may retain the overwritten nonbase value for lookup.
        received.receive(units[0]).unwrap();
        assert_eq!(received.activate(0, 0, 1).unwrap().pps.layer_id, 0);

        let mut original = LayeredParameters::from_configuration(&good, 32 << 20).unwrap();
        let old = original.activate(0, 0, 1).unwrap();
        let mut staged = original.clone();
        staged.receive(units[1]).unwrap();
        let error = staged.activate(0, 0, 1).unwrap_err().to_string();
        // The copied SPS RBSP is unchanged, but its latest NAL layer is invalid
        // for the base picture. PPS validation uses the same layer validator.
        assert_eq!(error, expected);
        assert!(Arc::ptr_eq(staged.active(0).unwrap(), &old));
        assert!(Arc::ptr_eq(original.active(0).unwrap(), &old));
        let repeated = original.activate(0, 0, 1).unwrap();
        assert_eq!(repeated.sps, old.sps);
        assert_eq!(repeated.pps, old.pps);
        staged.end_layer_sequence(0);
        assert_eq!(staged.activate(0, 0, 1).unwrap_err().to_string(), expected);
        assert!(staged.active(0).is_none());
    }
}

#[test]
fn shared_received_parameters_have_distinct_active_layer_bindings_and_reset() {
    let config = configuration("hevc-layered-reconstruct-shared-pps.mp4");
    let mut bank = LayeredParameters::from_configuration(&config, 32 << 20).unwrap();
    let base = bank.activate(0, 1, 1).unwrap();
    let enhancement = bank.activate(3, 1, 1).unwrap();
    assert!(Arc::ptr_eq(&base.vps, &enhancement.vps));
    assert_eq!(base.sps.sps.id, enhancement.sps.sps.id);
    assert_eq!(base.pps.pps, enhancement.pps.pps);
    assert_eq!([base.sps.layer_id, enhancement.sps.layer_id], [0, 3]);
    assert_eq!(
        [base.sps.nal_layer_id, enhancement.sps.nal_layer_id],
        [0, 0]
    );
    let before = bank.storage_bytes();
    let repeated = bank.activate(3, 1, 1).unwrap();
    assert_eq!(repeated.sps, enhancement.sps);
    assert_eq!(repeated.pps, enhancement.pps);
    assert_eq!(bank.storage_bytes(), before);
    bank.end_layer_sequence(3);
    assert!(bank.active(3).is_none());
    assert!(Arc::ptr_eq(bank.active(0).unwrap(), &base));
    let renewed = bank.activate(3, 1, 1).unwrap();
    assert!(!Arc::ptr_eq(&enhancement, &renewed));
    assert_eq!(renewed.sps, enhancement.sps);
    assert_eq!(renewed.pps, enhancement.pps);
}

#[test]
fn malformed_ids_missing_dependencies_and_budget_failures_preserve_state() {
    let config = configuration("hevc-layered-reconstruct-pcm-independent.mp4");
    assert!(LayeredParameters::from_configuration(&config, 1).is_err());
    let mut bank = LayeredParameters::from_configuration(&config, 32 << 20).unwrap();
    let old = bank.activate(0, 0, 1).unwrap();
    let before = bank.storage_bytes();
    for malformed in [
        &[0x44, 1, 0][..],
        &[0x44, 0, 0x80],
        &[0x44, 1, 0x02, 0x08],
        &[0x26, 1, 0x80],
    ] {
        assert!(bank.receive(malformed).is_err());
        assert_eq!(bank.storage_bytes(), before);
        assert!(Arc::ptr_eq(bank.active(0).unwrap(), &old));
    }
    assert_eq!(
        bank.activate(3, 63, 1).unwrap_err().to_string(),
        "HEVC active PPS is absent"
    );
    assert!(bank.activate(64, 0, 1).is_err());
    assert!(bank.activate(3, 1, usize::MAX).is_err());
    assert!(bank.active(3).is_none());
    assert!(Arc::ptr_eq(bank.active(0).unwrap(), &old));
    // A bank can receive syntax while lacking room for parsed metadata. The
    // failed activation must not publish a partial VPS/SPS/PPS binding.
    let received = LayeredParameters::from_configuration(&config, 32 << 20).unwrap();
    let mut tight =
        LayeredParameters::from_configuration(&config, received.storage_bytes() + 512).unwrap();
    assert!(tight.activate(0, 0, 1).is_err());
    assert!(tight.active(0).is_none());
    assert_eq!(tight.storage_bytes(), received.storage_bytes());
}

#[test]
fn sps_content_and_vps_updates_require_sequence_end_and_leave_old_snapshots_valid() {
    let initial = configuration("hevc-layered-reconstruct-pcm-independent.mp4");
    let updated = configuration("hevc-layered-sps-content-update.mp4");
    let config = HevcConfig::parse(&updated).unwrap();
    let updates = &config
        .arrays
        .iter()
        .find(|a| a.nal_type == 33)
        .unwrap()
        .units;
    assert_eq!(updates.len(), 3);
    let mut bank = LayeredParameters::from_configuration(&initial, 32 << 20).unwrap();
    let base = bank.activate(0, 0, 1).unwrap();
    let enhancement = bank.activate(3, 1, 1).unwrap();
    let constraints = base.sps.sps.profile.profile.unwrap().constraints;
    bank.receive(updates[1]).unwrap();
    assert_eq!(
        bank.activate(0, 0, 1).unwrap_err().to_string(),
        "HEVC active SPS changed before layer sequence end"
    );
    assert!(Arc::ptr_eq(bank.active(0).unwrap(), &base));
    assert_eq!(
        base.sps.sps.profile.profile.unwrap().constraints,
        constraints
    );
    bank.end_layer_sequence(0);
    let renewed = bank.activate(0, 0, 1).unwrap();
    assert_eq!(
        renewed.sps.sps.profile.profile.unwrap().constraints,
        constraints ^ (1 << 47)
    );
    assert!(Arc::ptr_eq(bank.active(3).unwrap(), &enhancement));
    assert_eq!(
        base.sps.sps.profile.profile.unwrap().constraints,
        constraints
    );
    let dependent = configuration("hevc-layered-reconstruct-pcm-dependent.mp4");
    let config = HevcConfig::parse(&dependent).unwrap();
    let vps = config
        .arrays
        .iter()
        .find(|a| a.nal_type == 32)
        .unwrap()
        .units[0];
    bank.receive(vps).unwrap();
    assert_eq!(
        bank.activate(0, 0, 1).unwrap_err().to_string(),
        "HEVC active VPS changed before sequence end"
    );
    bank.end_layer_sequence(0);
    assert_eq!(
        bank.activate(0, 0, 1).unwrap_err().to_string(),
        "HEVC active VPS changed before sequence end"
    );
    bank.end_layer_sequence(3);
    let next = bank.activate(0, 0, 1).unwrap();
    assert!(!Arc::ptr_eq(&next.vps, &base.vps));
    assert_eq!(
        next.sps.sps.profile.profile.unwrap().constraints,
        constraints ^ (1 << 47)
    );
    assert_eq!(
        base.sps.sps.profile.profile.unwrap().constraints,
        constraints
    );
}

#[test]
fn activation_budget_boundaries_never_publish_partial_parameters() {
    let config = configuration("hevc-layered-reconstruct-pcm-independent.mp4");
    let raw = LayeredParameters::from_configuration(&config, 32 << 20).unwrap();
    let mut failures = 0;
    let mut success = false;
    for extra in (512..80_000).step_by(193) {
        let budget = raw.storage_bytes() + extra;
        let mut bank = LayeredParameters::from_configuration(&config, budget).unwrap();
        match bank.activate(0, 0, 1) {
            Ok(base) => {
                assert!(bank.storage_bytes() <= budget);
                let before = bank.storage_bytes();
                match bank.activate(3, 1, 1) {
                    Ok(_) => {
                        assert!(bank.storage_bytes() <= budget);
                        success = true;
                        break;
                    }
                    Err(_) => {
                        assert!(bank.active(3).is_none());
                        assert!(Arc::ptr_eq(bank.active(0).unwrap(), &base));
                        assert_eq!(bank.storage_bytes(), before);
                    }
                }
            }
            Err(_) => {
                assert!(bank.active(0).is_none());
                assert_eq!(bank.storage_bytes(), raw.storage_bytes());
                failures += 1;
            }
        }
    }
    assert!(
        failures > 5,
        "must exercise several activation allocation stages"
    );
    assert!(success, "the finite original metadata must eventually fit");
}

#[test]
fn replacing_shared_sps_contents_waits_for_every_referencing_layer_sequence() {
    let initial = configuration("hevc-layered-reconstruct-shared-pps.mp4");
    let updated = configuration("hevc-layered-sps-content-update.mp4");
    let config = HevcConfig::parse(&updated).unwrap();
    let nal = config
        .arrays
        .iter()
        .find(|a| a.nal_type == 33)
        .unwrap()
        .units[1];
    let mut bank = LayeredParameters::from_configuration(&initial, 32 << 20).unwrap();
    let base = bank.activate(0, 1, 1).unwrap();
    let other = bank.activate(3, 1, 1).unwrap();
    bank.receive(nal).unwrap();
    bank.end_layer_sequence(0);
    assert_eq!(
        bank.activate(0, 1, 1).unwrap_err().to_string(),
        "HEVC active SPS changed before layer sequence end"
    );
    assert!(bank.active(0).is_none());
    assert!(Arc::ptr_eq(bank.active(3).unwrap(), &other));
    bank.end_layer_sequence(3);
    let next = bank.activate(0, 1, 1).unwrap();
    assert_ne!(next.sps.sps.profile.profile, base.sps.sps.profile.profile);
    assert_eq!(base.sps.sps.profile.profile, other.sps.sps.profile.profile);
}

#[test]
fn unchanged_enhancement_pps_resolves_new_reference_layer_scaling_on_activation() {
    let config = configuration("hevc-layered-activation-inference.mp4");
    let mut bank = LayeredParameters::from_configuration(&config, 32 << 20).unwrap();
    let base = bank.activate(0, 0, 1).unwrap();
    let old = bank.activate(3, 1, 1).unwrap();
    let first = |b: &fvid::codec::hevc_decoder::LayeredBinding| {
        b.pps
            .effective_scaling_lists
            .matrix(0, 0)
            .unwrap()
            .coefficients[0]
    };
    assert_eq!(first(&base), 17);
    assert_eq!(first(&old), 17);
    assert_eq!(old.pps.scaling_list_reference, Some(0));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    bank.receive(&std::fs::read(root.join("hevc-layered-pps-infer-from-pps.base.pps")).unwrap())
        .unwrap();
    let updated_base = bank.activate(0, 0, 1).unwrap();
    assert_eq!(first(&updated_base), 21);
    // Neither the received enhancement PPS nor its SPS changed. Inference
    // must still use the newly activated base PPS, not a stale cached binding.
    let updated = bank.activate(3, 1, 1).unwrap();
    assert_eq!(first(&updated), 21);
    let mut old_core = old.pps.pps.clone();
    // The core PPS stores the resolved scaling lists, so this derived field
    // must change even though its received syntax remains identical.
    assert_ne!(old_core.scaling_lists, updated.pps.pps.scaling_lists);
    old_core.scaling_lists = updated.pps.pps.scaling_lists.clone();
    assert_eq!(old_core, updated.pps.pps);
    assert_eq!(old.sps, updated.sps);
    assert_eq!(first(&old), 17);
    assert_eq!(first(&base), 17);
    bank.end_layer_sequence(0);
    assert_eq!(
        bank.activate(3, 1, 1).unwrap_err().to_string(),
        "HEVC PPS reference SPS is not active"
    );
    assert!(Arc::ptr_eq(bank.active(3).unwrap(), &updated));
}
