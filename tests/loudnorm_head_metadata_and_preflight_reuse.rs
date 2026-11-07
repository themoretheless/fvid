use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_path(name: &str) -> PathBuf {
    let idx = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "fvid-batch627-{}-{}-{}",
        std::process::id(),
        idx,
        name
    ))
}

fn write_stereo_wav_with_info(path: &std::path::Path, frames: usize, title: &str) {
    let mut out = Vec::new();
    let channels = 2u16;
    let sample_rate = 48_000u32;
    let bits_per_sample = 32u16;
    let byte_rate = sample_rate * u32::from(channels) * 4;
    let block_align = channels * 4;
    let data_bytes = (frames * usize::from(block_align)) as u32;

    let mut info_body = Vec::new();
    info_body.extend_from_slice(b"INFO");
    let mut inam_val = title.as_bytes().to_vec();
    inam_val.push(0);
    info_body.extend_from_slice(b"INAM");
    info_body.extend_from_slice(&(inam_val.len() as u32).to_le_bytes());
    info_body.extend_from_slice(&inam_val);
    if inam_val.len() % 2 != 0 {
        info_body.push(0);
    }

    let list_chunk_bytes = 8 + info_body.len() as u32;
    let riff_size = 36 + data_bytes + list_chunk_bytes;

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"LIST");
    out.extend_from_slice(&(info_body.len() as u32).to_le_bytes());
    out.extend_from_slice(&info_body);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..frames {
        let s = ((i as f32 * 0.05).sin() * 0.25).to_le_bytes();
        out.extend_from_slice(&s);
        out.extend_from_slice(&s);
    }
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&out).unwrap();
}

#[test]
fn loudnorm_wave_plan_linear_dynamic_and_matroska_metadata_reuse() {
    let src = temp_path("in.wav");
    let dst_lin = temp_path("out_lin.wav");
    let dst_dyn = temp_path("out_dyn.wav");
    let dst_mka = temp_path("out.mka");
    write_stereo_wav_with_info(&src, 4800, "Batch627Title");

    let mut opts = fvid::media::CopyOptions::default();
    opts.max_controlled_bytes = Some(64 * 1024 * 1024);

    let plan = fvid::media::plan_loudnorm(
        &src,
        Some("I=-16:TP=-1.5:LRA=11:print_format=summary"),
        true,
        &opts,
    )
    .expect("plan_loudnorm should succeed");
    assert_eq!(plan.command, "loudnorm");

    let lin_stats = fvid::media::apply_loudnorm(
        &src,
        &dst_lin,
        Some("I=-16:TP=-1.5:LRA=11:measured_I=-20:measured_TP=-6:measured_LRA=5:measured_thresh=-30:linear=true"),
        &opts,
    )
    .expect("linear loudnorm should succeed");
    assert_eq!(lin_stats.sample_frames, 4800);
    let lin_probe = fvid::media::probe(&dst_lin).expect("probe linear wav");
    assert_eq!(
        lin_probe.metadata.get("title").map(String::as_str),
        Some("Batch627Title")
    );

    let dyn_stats = fvid::media::apply_loudnorm(
        &src,
        &dst_dyn,
        Some("I=-16:TP=-1.5:LRA=11:linear=false"),
        &opts,
    )
    .expect("dynamic loudnorm should succeed");
    assert!(dyn_stats.sample_frames > 0);
    let dyn_probe = fvid::media::probe(&dst_dyn).expect("probe dynamic wav");
    assert_eq!(
        dyn_probe.metadata.get("title").map(String::as_str),
        Some("Batch627Title")
    );

    let mka_stats = fvid::media::apply_loudnorm(
        &src,
        &dst_mka,
        Some("I=-16:TP=-1.5:LRA=11:measured_I=-20:measured_TP=-6:measured_LRA=5:measured_thresh=-30:linear=true"),
        &opts,
    )
    .expect("matroska loudnorm should succeed");
    assert_eq!(mka_stats.sample_frames, 4800);
    let mka_probe = fvid::media::probe(&dst_mka).expect("probe mka");
    assert_eq!(
        mka_probe.metadata.get("title").map(String::as_str),
        Some("Batch627Title")
    );

    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst_lin);
    let _ = std::fs::remove_file(&dst_dyn);
    let _ = std::fs::remove_file(&dst_mka);
}
