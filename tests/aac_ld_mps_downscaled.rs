const CORPUS: &str = "aac-ld-mps-downscaled";
const CONTROLS: usize = 42;
const PROGRAMMES: usize = 112;
include!("support/aac_ld_mps_short_pcm.rs");

#[test]
fn invalid_short_geometry_and_spatial_ingress_modes_remain_explicit() {
    use fvid_media::owned_aac::aac_ld_mps_native::{Core, Decoder};
    let c = &m()["cases"][0];
    let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
    for factor in [0, 3, 5] {
        let core = Core {
            object_type: 23,
            sample_rate: 48000,
            frame_samples: 512,
            channels: 1,
            has_sbr: false,
            downscale: factor,
        };
        assert!(
            Decoder::new(&cfg, core)
                .unwrap_err()
                .to_string()
                .contains("invalid downscaled AAC LD/ELD spatial core geometry")
        );
    }
    let core = Core {
        object_type: 39,
        sample_rate: 48000,
        frame_samples: 480,
        channels: 1,
        has_sbr: true,
        downscale: 2,
    };
    let pcm = Decoder::new_with_sbr(&cfg, core, false).unwrap();
    assert!(!pcm.expects_shared_qmf());
    let mut third = core;
    third.downscale = 3;
    assert!(
        Decoder::new_with_sbr(&cfg, third, false)
            .unwrap_err()
            .to_string()
            .contains("LD-SBR downscale factor three")
    );
    let shared = Core {
        sample_rate: 24000,
        downscale: 1,
        ..core
    };
    assert!(
        Decoder::new_with_sbr(&cfg, shared, false)
            .unwrap()
            .expects_shared_qmf()
    );
    // Nominal-rate SSC needs a separate adaptation policy;do not silently retime it.
    let mut nominal = cfg.clone();
    nominal.sample_rate = 48000;
    let core = Core {
        object_type: 23,
        sample_rate: 48000,
        frame_samples: 480,
        channels: 1,
        has_sbr: false,
        downscale: 2,
    };
    assert!(
        Decoder::new(&nominal, core)
            .unwrap_err()
            .to_string()
            .contains("frame geometry disagree")
    );
}
