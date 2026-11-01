//! Optional exact-clock LD-MPS recomposition from explicitly supplied mono core PCM.
//! Original AAC core bits are parsed only to locate ancillary data. This is a
//! diagnostic stage boundary, not an independent waveform oracle or player path.
use fvid_media::owned_aac::{
    aac_channel::ChannelData,
    aac_ld_mps_decor::Mode,
    aac_ld_mps_native::{Core, Decoder, Stamp},
    bits::BitReader,
    config::AudioSpecificConfig,
};
use std::io::{Read, Write};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err(
            "usage: aac_ld_recompose_reference real|complex INPUT.mp4 CORE.f32le OUTPUT.f32le"
                .into(),
        );
    }
    let mode = match args[0].to_str() {
        Some("real") => Mode::Real,
        Some("complex") => Mode::Complex,
        _ => return Err("expected real|complex".into()),
    };
    let mut reader = fvid::container::mp4::Mp4Reader::open(
        std::io::BufReader::new(std::fs::File::open(&args[1])?),
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
        return Err("stage probe requires exactly one AAC track".into());
    }
    let track = tracks[0];
    let t = &reader.tracks()[track];
    t.ensure_constant_description()?;
    if !t.edits.is_empty() {
        return Err("stage probe requires an unedited track".into());
    }
    let config =
        AudioSpecificConfig::parse(fvid::codec::config::aac_specific_config(&t.configuration)?)?;
    if config.core.object_type != 23
        || config.core.channel_configuration != 1
        || config.protection.is_some()
        || config.separate_esc
        || config.sbr_present == Some(true)
        || config.eld_sbr.is_some()
    {
        return Err("stage probe requires mono ep0 AAC-LD".into());
    }
    let spatial = config
        .ld_mps
        .as_ref()
        .ok_or("missing LD-MPS configuration")?;
    let mut decoder = Decoder::new_with_decorrelation(
        spatial,
        Core {
            object_type: 23,
            sample_rate: config.core.sample_rate,
            frame_samples: config.core.frame_samples,
            channels: 1,
            has_sbr: false,
            downscale: 1,
        },
        None,
        mode,
    )?;
    let n = usize::from(config.core.frame_samples);
    let timescale = t.timescale;
    let count = t.samples.len();
    let channels = usize::from(spatial.tree.output_channels);
    let mut core = std::io::BufReader::new(std::fs::File::open(&args[2])?);
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[3])?);
    let mut bytes = vec![0; n * 4];
    let mut packet = vec![];
    let mut scalars = 0;
    for i in 0..count {
        core.read_exact(&mut bytes)?;
        let pcm: Vec<_> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        if pcm.iter().any(|v| !v.is_finite()) {
            return Err("nonfinite supplied core PCM".into());
        }
        let sample = reader.tracks()[track].samples.get(i).unwrap();
        let stamp = Stamp {
            pts: sample.pts,
            duration: u64::from(sample.duration),
            timescale,
        };
        reader.read_packet(track, i, &mut packet)?;
        let mut bits = BitReader::new(&packet);
        bits.read(4)?;
        ChannelData::read_ld(&mut bits, &config.core, false)?;
        if let Some(frame) = decoder.decode_core(&packet, bits.position(), &pcm, stamp)? {
            scalars += frame.samples.len();
            for v in frame.samples {
                out.write_all(&v.to_le_bytes())?;
            }
        }
    }
    if core.read(&mut [0; 1])? != 0 {
        return Err("extra core PCM after source EOF".into());
    }
    while let Some(frame) = decoder.finish()? {
        scalars += frame.samples.len();
        for v in frame.samples {
            out.write_all(&v.to_le_bytes())?;
        }
    }
    if scalars != count * n * channels {
        return Err("recomposed source sample count differs".into());
    }
    out.flush()?;
    println!(
        "mode={mode:?} packets={count} rate={} channels={channels} scalars={scalars}",
        spatial.sample_rate
    );
    Ok(())
}
