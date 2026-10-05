//! Explicit benchmark-only FFmpeg comparison for high-depth camera conversion.
use fvid::{
    playback_native::NativeReader,
    virtual_camera::{CameraTick, LatestFrame, NativeCameraSource},
};
use std::{io::Cursor, process::Command, time::Instant};
fn main() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for depth in [10, 16] {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/camera-y4m-{depth}.y4m"
        ));
        let start = Instant::now();
        let reference=Command::new(&ffmpeg).args(["-nostdin","-v","error","-i"]).arg(&input)
            .args(["-vf","scale=in_color_matrix=bt601:in_range=tv:out_range=pc:flags=neighbor+bitexact:sws_dither=none","-f","rawvideo","-pix_fmt","rgb24","-"])
            .output().expect("benchmark requires FFmpeg");
        let reference_time = start.elapsed();
        assert!(
            reference.status.success(),
            "{}",
            String::from_utf8_lossy(&reference.stderr)
        );
        assert_eq!(reference.stdout.len(), 384);
        let data = std::fs::read(input).unwrap();
        let start = Instant::now();
        let reader = NativeReader::software(Cursor::new(data), 16 << 20).unwrap();
        let mut source = NativeCameraSource::new(reader);
        let destination = LatestFrame::new(8, 8, 256).unwrap();
        let mut actual = [0; 256];
        let mut peak = 0;
        for index in 0..2 {
            let tick = CameraTick {
                sequence: index,
                host_time_ns: index + 1,
                media_time_ns: index * 40_000_000,
            };
            assert!(source.publish(tick, &destination).unwrap());
            destination.copy_latest(None, &mut actual).unwrap();
            let rgb = &reference.stdout[index as usize * 192..(index as usize + 1) * 192];
            for (bgra, rgb) in actual.as_chunks::<4>().0.iter().zip(rgb.as_chunks::<3>().0.iter()) {
                assert_eq!(bgra[3], 255);
                for channel in 0..3 {
                    peak = peak.max(bgra[2 - channel].abs_diff(rgb[channel]));
                }
            }
        }
        assert!(peak <= 3, "FFmpeg conversion mismatch: {peak}");
        println!("{depth}-bit two-frame camera: owned {:?}, FFmpeg {:?}, max RGB difference {peak} (includes initialization/process startup)",start.elapsed(),reference_time);
    }
}
