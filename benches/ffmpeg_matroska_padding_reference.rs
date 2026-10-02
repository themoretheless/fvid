//! Explicit diagnostic of Matroska AAC padding against an external decoder.
use fvid::container::{adts, matroska_write, webm};
use std::path::{Path, PathBuf};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-mka-mux-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio")
        .join(name)
}
fn inspect_padding_reference() {
    let d = dir("padding-reference");
    use matroska_write::{Encoding, PacketWriter, TrackOptions, TrackSpec};
    let data = std::fs::read(fixture("aac-mono-44k.aac")).unwrap();
    let source = adts::Aac::parse(&data, &Default::default()).unwrap();
    let ns = |samples: u64| {
        (samples * 1_000_000_000 + u64::from(source.sample_rate) / 2)
            / u64::from(source.sample_rate)
    };
    let mut original = Vec::new();
    fvid::native_media::decode_aac_pcm(&data, &mut original, &Default::default()).unwrap();
    for (delay, head, tail) in [(128, 0, 256), (0, 128, 256)] {
        let mut output = std::io::Cursor::new(Vec::new());
        let mut writer = PacketWriter::new_with_options(
            &mut output,
            &[TrackSpec {
                encoding: Encoding::Aac {
                    configuration: &source.frames[0].asc,
                    sample_rate: source.sample_rate,
                    channels: source.channels,
                },
                name: "",
                language: "und",
            }],
            &[TrackOptions {
                codec_delay_ns: ns(delay),
                ..Default::default()
            }],
        )
        .unwrap();
        for i in 0..source.packets() {
            let padding = if i == 0 {
                -(ns(head) as i64)
            } else if i + 1 == source.packets() {
                ns(tail) as i64
            } else {
                0
            };
            writer
                .write_packet_with_padding(
                    0,
                    ns(i as u64 * 1024),
                    ns((i as u64 + 1) * 1024) - ns(i as u64 * 1024),
                    true,
                    source.packet(i),
                    padding,
                )
                .unwrap();
        }
        writer.finish().unwrap();
        let mut reader =
            webm::WebmReader::open(std::io::Cursor::new(output.get_ref()), Default::default())
                .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks[0].codec_delay_ns, ns(delay));
        assert_eq!(reader.packets[0].discard_padding_ns, -(ns(head) as i64));
        assert_eq!(
            reader.packets.last().unwrap().discard_padding_ns,
            ns(tail) as i64
        );
        assert_eq!(
            reader.duration_ns,
            Some(ns(source.packets() as u64 * 1024) - ns(delay) - ns(tail))
        );
        let mut decoded = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(output.get_ref(), &mut decoded, None)
            .unwrap();
        let stride = usize::from(source.channels) * 4;
        let path = d.0.join(format!("padding-{delay}-{head}-{tail}.mka"));
        std::fs::write(&path, output.get_ref()).unwrap();
        let reference = std::process::Command::new(std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap())
            .args(["-v", "error", "-i"]).arg(path)
            .args(["-map", "0:a:0", "-f", "f32le", "-c:a", "pcm_f32le", "pipe:1"])
            .output().unwrap();
        assert!(reference.status.success(), "{}", String::from_utf8_lossy(&reference.stderr));
        assert_eq!(decoded.len(), reference.stdout.len(), "padding sample count differs from reference");
        println!("delay={delay} head={head} tail={tail}: owned={} reference={} cropped={} sample frames", decoded.len()/stride, reference.stdout.len()/stride, original.len()/stride-delay.max(head) as usize-tail as usize);
    }
}

fn main() { inspect_padding_reference(); }
