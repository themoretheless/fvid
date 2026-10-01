//! Regenerate synthetic seek inputs using owned MP4/ADTS/Matroska framing.
use fvid::{
    codec::config::{AacConfig, aac_specific_config},
    container::{adts::StreamReader, matroska_write, mp4::Mp4Reader},
};
use std::{
    fs::File,
    io::{BufReader, Cursor, Write},
    path::PathBuf,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: aac_seek_fixture SOURCE.mp4 OUTPUT_DIRECTORY".into());
    }
    let mut reader = Mp4Reader::open(BufReader::new(File::open(&args[0])?), Default::default())?;
    let index = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun" && t.codec == *b"mp4a")
        .ok_or("no AAC track")?;
    let track = &reader.tracks()[index];
    let config = AacConfig::parse(aac_specific_config(&track.configuration)?)?;
    let rates = [
        96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
    ];
    let rate = rates
        .iter()
        .position(|r| *r == config.sample_rate)
        .ok_or("ADTS rate unavailable")? as u8;
    if config.object_type != 2 || config.channel_configuration == 0 || config.frame_samples != 1024
    {
        return Err("requires fixed-layout AAC-LC with 1024 samples".into());
    }
    let count = track.samples.len();
    let directory = PathBuf::from(&args[1]);
    let path = directory.join("aac-seek-checkpoints.aac");
    let mut output = File::create(&path)?;
    let mut packet = Vec::new();
    for sample in 0..count {
        reader.read_packet(index, sample, &mut packet)?;
        let size = packet
            .len()
            .checked_add(7)
            .filter(|n| *n <= 8191)
            .ok_or("ADTS packet too large")?;
        let channels = config.channel_configuration;
        let header = [
            0xff,
            0xf1,
            0x40 | (rate << 2) | (channels >> 2),
            ((channels & 3) << 6) | ((size >> 11) as u8),
            (size >> 3) as u8,
            ((size & 7) as u8) << 5 | 0x1f,
            0xfc,
        ];
        output.write_all(&header)?;
        output.write_all(&packet)?;
    }
    output.flush()?;
    let input = StreamReader::open(BufReader::new(File::open(path)?))?;
    matroska_write::write_adts(
        input,
        &mut File::create(directory.join("aac-seek-checkpoints.mka"))?,
        None,
        None,
    )?;
    let mut adts_pcm = Vec::new();
    let mut matroska_pcm = Vec::new();
    fvid::native_media::decode_adts_aac_reader(
        StreamReader::open(Cursor::new(std::fs::read(
            directory.join("aac-seek-checkpoints.aac"),
        )?))?,
        &mut adts_pcm,
        None,
    )?;
    fvid::native_media::decode_matroska_aac_reader(
        fvid::container::webm::WebmReader::open(
            Cursor::new(std::fs::read(directory.join("aac-seek-checkpoints.mka"))?),
            Default::default(),
        )?,
        &mut matroska_pcm,
        None,
    )?;
    if adts_pcm.is_empty() || adts_pcm != matroska_pcm {
        return Err("generated ADTS/Matroska PCM differs".into());
    }
    println!(
        "Verified {count} packets and {} PCM bytes in both containers",
        adts_pcm.len()
    );
    Ok(())
}
