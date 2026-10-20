//! Explicit native MIDI CPU benchmark; file loading and PCM checksums are outside timing.
#[cfg(feature = "player")]
fn main() {
    use fvid::audio::{AudioDecode, AudioPacket, AudioStream, EncodedPacket};
    use fvid::codec::midi_decoder::MidiDecoder;
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
        hint::black_box,
        io::Cursor,
        path::PathBuf,
        time::Instant,
    };
    fn render(
        engine: &mut MidiDecoder,
        packets: &[EncodedPacket],
        mut consume: impl FnMut(&AudioPacket),
    ) -> usize {
        engine.reset();
        let mut bytes = 0;
        for p in packets {
            let f = engine
                .decode_packet(black_box(&p.data), p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            bytes += f.packet.data.len();
            consume(&f.packet);
        }
        bytes
    }
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("iterations or dump");
    let dump = if mode == "dump" {
        Some(PathBuf::from(args.next().expect("dump directory")))
    } else {
        None
    };
    let iterations = if dump.is_some() {
        0
    } else {
        mode.parse::<usize>().unwrap()
    };
    let paths = args.map(PathBuf::from).collect::<Vec<_>>();
    assert!(!paths.is_empty());
    let inputs = paths
        .iter()
        .map(|path| {
            let mut source = fvid::playback_smf::SmfAudioReader::open(
                Cursor::new(std::fs::read(path).unwrap()),
                Default::default(),
            )
            .unwrap();
            assert_eq!((source.sample_rate(), source.channels()), (44100, 2));
            let mut packets = Vec::new();
            while let Some(p) = source.next_packet().unwrap() {
                packets.push(p);
            }
            packets
        })
        .collect::<Vec<_>>();
    let mut engines = (0..inputs.len())
        .map(|_| MidiDecoder::new(44100, 2).unwrap())
        .collect::<Vec<_>>();
    let mut hashes = Vec::new();
    for ((path, packets), engine) in paths.iter().zip(&inputs).zip(&mut engines) {
        let mut hash = DefaultHasher::new();
        let mut pcm = Vec::new();
        let bytes = render(engine, packets, |p| {
            p.data.hash(&mut hash);
            p.pts.hash(&mut hash);
            p.timebase_num.hash(&mut hash);
            p.timebase_den.hash(&mut hash);
            if dump.is_some() {
                pcm.extend_from_slice(&p.data);
            }
        });
        hashes.push(serde_json::json!({"input":path,"bytes":bytes,"checksum64":format!("{:016x}",hash.finish())}));
        if let Some(directory) = &dump {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(
                directory
                    .join(path.file_name().unwrap())
                    .with_extension("f32le"),
                pcm,
            )
            .unwrap();
        }
    }
    for _ in 0..5 {
        for (engine, packets) in engines.iter_mut().zip(&inputs) {
            render(engine, packets, |p| {
                black_box(p);
            });
        }
    }
    let start = Instant::now();
    let mut total = 0usize;
    for _ in 0..iterations {
        for (engine, packets) in engines.iter_mut().zip(&inputs) {
            total += render(engine, packets, |p| {
                black_box(p);
            });
        }
    }
    println!(
        "{}",
        serde_json::json!({"iterations":iterations,"inputs":inputs.len(),"decoder_object_bytes":std::mem::size_of::<MidiDecoder>(),"elapsed_ns":start.elapsed().as_nanos(),"pcm_bytes":total,"outside_timing_pcm_controls":hashes,"scope":"Native MIDI decoder CPU render only; containers/fileIO/checksums excluded; not a device/player-latency measurement."})
    );
}
#[cfg(not(feature = "player"))]
fn main() {
    eprintln!("midi_render_probe requires --features player");
}
