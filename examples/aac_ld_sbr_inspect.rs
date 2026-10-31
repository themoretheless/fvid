//! Optional offline syntax/energy diagnostic for ordinary mono ELD FVIDAAC1
//! records exported by aac_spatial_reference. Strict noise0..30 diagnostics.
//! Not a waveform conformance oracle.
use fvid_media::owned_aac::{
    aac_channel::ChannelData, aac_sbr_data::Data, aac_sbr_header::Header, aac_sbr_history::History,
    bits::BitReader, config::AudioSpecificConfig,
};
use std::io::Read;

fn word(input: &mut impl Read) -> std::io::Result<usize> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes) as usize)
}
fn payload(input: &mut impl Read, limit: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let n = word(input)?;
    if n == 0 || n > limit {
        return Err("FVIDAAC1 payload admission bound".into());
    }
    let mut data = vec![0; n];
    input.read_exact(&mut data)?;
    Ok(data)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err("usage: aac_ld_sbr_inspect INPUT.records".into());
    }
    let mut input = std::io::BufReader::new(std::fs::File::open(&args[0])?);
    let mut magic = [0; 8];
    input.read_exact(&mut magic)?;
    if &magic != b"FVIDAAC1" {
        return Err("invalid FVIDAAC1 magic".into());
    }
    let asc = payload(&mut input, 65536)?;
    let config = AudioSpecificConfig::parse(&asc)?;
    if config.core.object_type != 39
        || config.core.channel_configuration != 1
        || config.protection.is_some()
        || config.separate_esc
        || config.ld_mps.is_some()
        || config.eld_downscale_sample_rate.is_some()
    {
        return Err("diagnostic requires ordinary mono ELD/LD-SBR".into());
    }
    let sbr = config.eld_sbr.ok_or("missing LD-SBR configuration")?;
    let mut header = sbr.headers[0].clone();
    let mut history = History::default();
    let count = word(&mut input)?;
    if count == 0 || count > 1_000_000 {
        return Err("FVIDAAC1 AU count admission bound".into());
    }
    for index in 0..count {
        let packet = payload(&mut input, 1024 * 1024)?;
        let mut bits = BitReader::new(&packet);
        let core = ChannelData::read_eld(&mut bits, &config.core, None, false)?;
        let core_bits = bits.position();
        let expected_crc = if sbr.crc { Some(bits.read(10)?) } else { None };
        let mut crc_input = bits.clone();
        if bits.bit()? {
            header = Header::read(&mut bits, packet.len() * 8)?;
        }
        let slots = (config.core.frame_samples / 32) as u8;
        let data = Data::read_low_delay(
            &mut bits,
            packet.len() * 8,
            &header,
            config
                .core
                .sample_rate
                .checked_mul(2)
                .ok_or("SBR rate overflow")?,
            slots,
            1,
        )?;
        let end = bits.position();
        if let Some(expected) = expected_crc {
            let mut crc = 0u32;
            while crc_input.position() < end {
                let feedback = ((crc >> 9) ^ crc_input.read(1)?) & 1;
                crc = ((crc << 1) & 0x3ff) ^ if feedback != 0 { 0x233 } else { 0 };
            }
            if crc != expected {
                return Err("LD-SBR diagnostic CRC mismatch".into());
            }
        }
        let parameters = history.decode(&data, header.amplitude_resolution, slots)?;
        println!(
            "{}",
            serde_json::json!({
                "frame":index,"core_bits":core_bits,"sbr_end_bits":end,
                "core_max_sfb":core.info.max_sfb,
            "core_peak_quantized":core.quantized.iter().map(|v| i32::from(*v).abs()).max().unwrap_or(0),
                "header":format!("{header:?}"),"frequency":format!("{:?}",data.frequency),
                "grid":format!("{:?}",data.channels[0].grid),
                "harmonics":data.channels[0].harmonics,
                "inverse_filter":format!("{:?}",data.channels[0].inverse_filter),
                "envelope":parameters.channels[0].envelope,"noise":parameters.channels[0].noise,
                "quantized_envelope":parameters.channels[0].quantized_envelope,
                "quantized_noise":parameters.channels[0].quantized_noise
            })
        );
    }
    if input.read(&mut [0; 1])? != 0 {
        return Err("trailing FVIDAAC1 bytes".into());
    }
    Ok(())
}
