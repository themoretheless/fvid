//! Decode unedited, constant-description MP4 AAC packets with either owned
//! spatial decorrelator. Usage: aac_spatial_reference real|complex|packets INPUT OUTPUT.
//! `packets` writes bounded length-prefixed ASC/AUs for optional external probes.
//! Packet export intentionally ignores container edits; it exports no PCM/timeline.
//! This numerical reference probe publishes raw decoder PCM, not player edits,
//! priming compensation, a delay search or an automatic conformance verdict.
use fvid_media::owned_aac::{AacDecodeOptions, NativeAacDecoder, aac_ld_mps_decor::Mode};
use std::io::{BufReader, BufWriter, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: aac_spatial_reference real|complex|packets INPUT.mp4 OUTPUT".into());
    }
    let mode = match args[0].to_str() {
        Some("real") => Some(Mode::Real),
        Some("complex") => Some(Mode::Complex),
        Some("packets") => None,
        _ => return Err("expected real, complex or packets".into()),
    };
    let mut reader = fvid::container::mp4::Mp4Reader::open(
        BufReader::new(std::fs::File::open(&args[1])?),
        Default::default(),
    )?;
    let tracks: Vec<_> = reader
        .tracks()
        .iter()
        .enumerate()
        .filter(|(_, t)| t.handler == *b"soun" && t.codec == *b"mp4a")
        .map(|(i, _)| i)
        .collect();
    if tracks.len() != 1 {
        return Err("reference probe requires exactly one AAC track".into());
    }
    let index = tracks[0];
    let track = &reader.tracks()[index];
    track.ensure_constant_description()?;
    let ignored_edits = track.edits.len();
    if mode.is_some() && ignored_edits != 0 {
        return Err("reference probe requires an unedited AAC track".into());
    }
    let count = track.samples.len();
    let timescale = track.timescale;
    let asc = fvid::codec::config::aac_specific_config(&track.configuration)?;
    if mode.is_none() {
        let mut output = BufWriter::new(std::fs::File::create(&args[2])?);
        output.write_all(b"FVIDAAC1")?;
        output.write_all(&u32::try_from(asc.len())?.to_le_bytes())?;
        output.write_all(asc)?;
        output.write_all(&u32::try_from(count)?.to_le_bytes())?;
        let mut packet = Vec::new();
        for i in 0..count {
            reader.read_packet(index, i, &mut packet)?;
            output.write_all(&u32::try_from(packet.len())?.to_le_bytes())?;
            output.write_all(&packet)?;
        }
        output.flush()?;
        println!("packets={count} format=FVIDAAC1 ignored_edits={ignored_edits}");
        return Ok(());
    }
    let mode = mode.unwrap();
    let mut decoder = NativeAacDecoder::new_with_options(
        asc,
        AacDecodeOptions {
            output_sample_rate: Some(track.sample_rate),
            spatial_decorrelation: mode,
            ..Default::default()
        },
    )?;
    let mut output = BufWriter::new(std::fs::File::create(&args[2])?);
    let mut samples = 0usize;
    let mut write = |pcm: Vec<f32>| -> std::io::Result<()> {
        samples += pcm.len();
        for v in pcm {
            output.write_all(&v.to_le_bytes())?;
        }
        Ok(())
    };
    let mut packet = Vec::new();
    for i in 0..count {
        let sample = reader.tracks()[index].samples.get(i).unwrap();
        reader.read_packet(index, i, &mut packet)?;
        if let Some(frame) = decoder.decode_timed_in_clock(
            &packet,
            sample.pts,
            u64::from(sample.duration),
            timescale,
        )? {
            write(frame.samples)?;
        }
    }
    while let Some(frame) = decoder.finish()? {
        write(frame.samples)?;
    }
    output.flush()?;
    println!(
        "mode={mode:?} packets={count} rate={} channels={} scalars={samples}",
        decoder.sample_rate(),
        decoder.channels()
    );
    Ok(())
}
