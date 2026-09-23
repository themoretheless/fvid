// Throwaway stage profiler: repeatedly software-decodes a fixture so an
// external sampling profiler (`sample`) can attribute time inside the decoder.
use fvid::container::mp4::{Limits, Mp4Reader};
use fvid::playback_mp4::Mp4VideoReader;
use fvid::playback_webm::WebmVideoReader;
use std::io::BufReader;
use std::time::Instant;

fn mp4(path: &str, budget: usize) {
    for _ in 0..400 {
        let file = std::fs::File::open(path).unwrap();
        let demux = Mp4Reader::open(BufReader::new(file), Limits::default()).unwrap();
        let index = demux
            .tracks()
            .iter()
            .position(|t| t.handler == *b"vide")
            .unwrap();
        let mut reader = Mp4VideoReader::from_demuxer(demux, index, budget).unwrap();
        while reader.read_frame().unwrap().is_some() {}
    }
}

fn webm(path: &str, budget: usize) {
    for _ in 0..400 {
        let file = std::fs::File::open(path).unwrap();
        let mut reader = WebmVideoReader::open(BufReader::new(file), budget).unwrap();
        while reader.read_frame().unwrap() {}
    }
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "hevc".into());
    let started = Instant::now();
    match which.as_str() {
        "hevc" => mp4("tests/fixtures/hevc/main-ipb.mp4", 256 << 20),
        "hevc10" => mp4("tests/fixtures/hevc/main10-ipb.mp4", 256 << 20),
        "av1" => webm("tests/fixtures/av1/random-access.webm", 256 << 20),
        "vp9" => webm("tests/fixtures/vp9/adaptive-tiles.webm", 256 << 20),
        other => panic!("unknown target {other}"),
    }
    eprintln!("{which} profile loop: {:?}", started.elapsed());
}
