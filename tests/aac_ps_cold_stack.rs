//! Cold QMF initialization must fit a normal worker stack, without a warmed cache.
#[test]
fn cold_ps_export_fits_two_mib_stack() {
    const CHILD: &str = "FVID_COLD_PS_STACK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","cold_ps_export_fits_two_mib_stack","--nocapture"])
            .env(CHILD,"1").status().unwrap();
        assert!(status.success(),"cold PS export failed: {status}");
        return;
    }
    std::thread::Builder::new().name("cold-ps-export".into()).stack_size(2*1024*1024).spawn(|| {
        // Leave caller headroom: the reported selection/export call retains
        // additional plans and controls above the cold synthesis invocation.
        let caller_frame=[0u8;128*1024];
        std::hint::black_box(&caller_frame);
        let source=std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/aac-ps-cold-stack/late-ps.mp4");
        let output=std::env::temp_dir().join(format!("fvid-cold-ps-{}.f32le",std::process::id()));
        let _=std::fs::remove_file(&output);
        let stats=fvid::native_export::export_audio_pcm_selected(&source,&output,None,1.0,None,None,None,None,None).unwrap();
        assert_eq!((stats.sample_rate,stats.channels,stats.sample_frames),(48000,2,9600));
        let pcm=std::fs::read(&output).unwrap();std::fs::remove_file(&output).unwrap();
        assert_eq!(pcm.len(),9600*2*4);
        let samples:Vec<f32>=pcm.chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect();
        assert!(samples.iter().all(|v|v.is_finite()));
        assert!(samples.iter().any(|v|v.abs()>1e-8));
        assert!(samples[..1600*2].iter().all(|v|*v==0.));
        assert!(samples[8000*2..].iter().all(|v|*v==0.));
        std::hint::black_box(&caller_frame);
    }).unwrap().join().unwrap();
}
