//! Measure the GUI's native reader without presentation pacing or file output.
//! CPU RGB conversion is included only in `rgb` mode; this is not GPU render timing.
use std::{fs::File, io::BufReader, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !(2..=4).contains(&args.len()) {
        return Err("usage: native_decode_bench INPUT raw|rgb [FRAME_LIMIT] [software|auto]".into());
    }
    let rgb = match args[1].to_str() {
        Some("rgb") => true,
        Some("raw") => false,
        _ => return Err("mode must be raw or rgb".into()),
    };
    let limit: usize = args.get(2).map(|s|s.to_str().ok_or("invalid frame limit")?.parse::<usize>().map_err(|_|"invalid frame limit")).transpose()?.unwrap_or(300);
    if limit==0 {return Err("frame limit must be positive".into());}
    let software=match args.get(3).and_then(|s|s.to_str()).unwrap_or("software") {
        "software"=>true,"auto"=>false,_=>return Err("backend must be software or auto".into()),
    };
    let started=Instant::now();
    let input=BufReader::new(File::open(&args[0])?);
    let mut reader=if software {fvid::playback_native::NativeReader::software(input,256<<20)?} else {fvid::playback_native::NativeReader::new(input,256<<20)?};
    let open_ms=started.elapsed().as_secs_f64()*1000.;
    let mut samples=Vec::new();
    let started=Instant::now();
    while samples.len()<limit {
        let frame_start=Instant::now();
        let exists=if rgb {reader.read_frame()?} else {reader.read_frame_raw()?.is_some()};
        if !exists {break;}
        samples.push(frame_start.elapsed().as_secs_f64()*1000.);
    }
    let seconds=started.elapsed().as_secs_f64();
    if samples.is_empty() {return Err("no frames decoded".into());}
    let first_ms=samples[0];
    let over_budget=samples.iter().filter(|&&v|v>1000./60.).count();
    samples.sort_by(f64::total_cmp);
    let percentile=|p:usize|samples[((samples.len()*p).div_ceil(100)-1).min(samples.len()-1)];
    println!("{}",serde_json::json!({
        "mode":if rgb {"decode_cpu_rgb"} else {"decode_raw"},
        "backend_request":if software {"software"} else {"auto"},
        "frames":samples.len(),"dimensions":reader.dimensions(),"open_ms":open_ms,
        "elapsed_seconds":seconds,"throughput_fps":samples.len() as f64/seconds,
        "first_frame_ms":first_ms,"p50_ms":percentile(50),"p95_ms":percentile(95),
        "p99_ms":percentile(99),"max_ms":samples.last(),"frames_over_60fps_budget":over_budget,
        "includes_gpu_render":false,"includes_presentation_pacing":false
    }));
    Ok(())
}
