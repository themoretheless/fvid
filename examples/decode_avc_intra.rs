//! Decode the currently supported AVC intra subset using only FVid.
use fvid::{
    codec::{
        avc::{Pps, Sps},
        avc_picture::decode_intra_picture,
        avc_slice::SliceHeader,
        config::{AvcConfig, NalUnits},
    },
    container::mp4::{Limits, Mp4Reader},
};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: decode_avc_intra INPUT.mp4 OUTPUT.yuv".into());
    }
    let mut mp4 = Mp4Reader::open(BufReader::new(File::open(&args[0])?), Limits::default())?;
    let index = mp4
        .tracks()
        .iter()
        .position(|t| matches!(&t.codec, b"avc1" | b"avc3"))
        .ok_or("no AVC track")?;
    let config = AvcConfig::parse(&mp4.tracks()[index].configuration)?;
    let length_size = config.length_size;
    let sps: Vec<_> = config
        .sps
        .iter()
        .map(|nal| Sps::parse(nal))
        .collect::<fvid::Result<_>>()?;
    let mut pairs = Vec::new();
    for nal in config.pps {
        let pair = sps
            .iter()
            .find_map(|sps| Pps::parse(nal, sps).ok().map(|pps| (sps.clone(), pps)))
            .ok_or("invalid PPS or missing SPS")?;
        pairs.push(pair);
    }
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[1])?,
    );
    let mut packet = Vec::new();
    let mut frames = 0;
    for sample in 0..mp4.tracks()[index].samples.len() {
        mp4.read_packet(index, sample, &mut packet)?;
        for nal in NalUnits::new(&packet, length_size)? {
            let nal = nal?;
            if matches!(nal[0] & 31, 1 | 5) {
                let id = SliceHeader::parameter_set_id(nal)?;
                let (sps, pps) = pairs
                    .iter()
                    .find(|(_, p)| p.id == id)
                    .ok_or("unknown PPS")?;
                let header = SliceHeader::parse(nal, sps, pps)?;
                let picture = decode_intra_picture(&header, sps, pps, 256 << 20)?;
                picture.write_planar(&mut output)?;
                frames += 1;
            } else if !matches!(nal[0] & 31, 6 | 9 | 12) {
                return Err("unsupported in-band AVC NAL".into());
            }
        }
    }
    output.flush()?;
    println!("frames={frames}");
    Ok(())
}
