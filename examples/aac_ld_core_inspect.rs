//! Optional offline AAC-LD mono core syntax diagnostic, not a PCM oracle.
//! Ancillary/spatial suffixes are not interpreted by this diagnostic.
use fvid_media::owned_aac::{
    aac_channel::ChannelData, bits::BitReader, config::AudioSpecificConfig,
};
use std::io::Read;
fn word(input: &mut impl Read) -> std::io::Result<usize> {
    let mut b = [0; 4];
    input.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b) as usize)
}
fn payload(input: &mut impl Read, limit: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let n = word(input)?;
    if n == 0 || n > limit {
        return Err("record admission bound".into());
    }
    let mut data = vec![0; n];
    input.read_exact(&mut data)?;
    Ok(data)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err("usage: aac_ld_core_inspect INPUT.records".into());
    }
    let mut input = std::io::BufReader::new(std::fs::File::open(&args[0])?);
    let mut magic = [0; 8];
    input.read_exact(&mut magic)?;
    if &magic != b"FVIDAAC1" {
        return Err("record magic".into());
    }
    let asc = payload(&mut input, 65536)?;
    let config = AudioSpecificConfig::parse(&asc)?;
    if config.core.object_type != 23
        || config.core.channel_configuration != 1
        || config.protection.is_some()
        || config.separate_esc
    {
        return Err("diagnostic requires mono AAC-LD core".into());
    }
    let count = word(&mut input)?;
    if count == 0 || count > 1_000_000 {
        return Err("AU count admission bound".into());
    }
    for index in 0..count {
        let packet = payload(&mut input, 1_048_576)?;
        let mut bits = BitReader::new(&packet);
        let tag = bits.read(4)?;
        let (core, ltp) = ChannelData::read_ld(&mut bits, &config.core, false)?;
        println!(
            "{}",
            serde_json::json!({"frame":index,"frame_samples":config.core.frame_samples,"tag":tag,"max_sfb":core.info.max_sfb,"window_shape":if core.info.shape == fvid_media::owned_aac::aac_synthesis::WindowShape::Sine {"Sine"} else {"LowOverlap"},"ltp":format!("{ltp:?}"),"has_ltp":ltp.is_some(),"spatial":config.ld_mps.is_some(),"tns":core.tns.is_some(),"books":core.codebooks,"core_bits":bits.position()})
        );
    }
    if input.read(&mut [0; 1])? != 0 {
        return Err("trailing records".into());
    }
    Ok(())
}
