//! Inspect FVid's MP4 packet index without linking a codec library.
use fvid::codec::config::{AacConfig, AvcConfig, HevcConfig, NalUnits, aac_specific_config};
use fvid::container::mp4::{Limits, Mp4Reader};
use std::{fs::File, io::BufReader};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: mp4_packets INPUT.mp4")?;
    let mut mp4 = Mp4Reader::open(BufReader::new(File::open(path)?), Limits::default())?;
    let mut lengths = Vec::new();
    let mut avc_parameters = Vec::new();
    for track in mp4.tracks() {
        let mut pairs = Vec::new();
        let length = match &track.codec {
            b"avc1" | b"avc3" => {
                let config = AvcConfig::parse(&track.configuration)?;
                let mut parameter_sets = Vec::new();
                for nal in &config.sps {
                    let sps = fvid::codec::avc::Sps::parse(nal)?;
                    let (width, height) = sps.display_dimensions();
                    eprintln!(
                        "AVC_SPS,{width},{height},{},{},{}",
                        sps.profile, sps.bit_depth_luma, sps.chroma_format
                    );
                    parameter_sets.push(sps);
                }
                for nal in &config.pps {
                    let pair = parameter_sets
                        .iter()
                        .find_map(|sps| {
                            fvid::codec::avc::Pps::parse(nal, sps)
                                .ok()
                                .map(|pps| (sps.clone(), pps))
                        })
                        .ok_or("AVC PPS is invalid or references a missing SPS")?;
                    pairs.push(pair);
                }
                Some(config.length_size)
            }
            b"hvc1" | b"hev1" => Some(HevcConfig::parse(&track.configuration)?.length_size),
            b"mp4a" => {
                let config = AacConfig::parse(aac_specific_config(&track.configuration)?)?;
                eprintln!(
                    "AAC: {} Hz, {} channels",
                    config.sample_rate, config.channels
                );
                None
            }
            _ => None,
        };
        lengths.push(length);
        avc_parameters.push(pairs);
    }
    let mut packet = Vec::new();
    for (track, length) in lengths.into_iter().enumerate() {
        let mut poc = fvid::codec::avc_poc::PocDecoder::new();
        let mut references = None;
        if let Some(length) = length {
            for sample in 0..mp4.tracks()[track].samples.len() {
                mp4.read_packet(track, sample, &mut packet)?;
                let mut first_slice = true;
                for nal in NalUnits::new(&packet, length)? {
                    let nal = nal?;
                    if !avc_parameters[track].is_empty() && matches!(nal[0] & 31, 1 | 5) {
                        use fvid::codec::avc_slice::SliceHeader;
                        let id = SliceHeader::parameter_set_id(nal)?;
                        let (sps, pps) = avc_parameters[track]
                            .iter()
                            .find(|(_, p)| p.id == id)
                            .ok_or("unknown slice PPS")?;
                        let header = SliceHeader::parse(nal, sps, pps)?;
                        if first_slice {
                            let order = poc.decode(sps, &header)?;
                            if references.is_none() || header.idr {
                                references =
                                    Some(fvid::codec::avc_dpb::ReferenceBuffer::<()>::new(
                                        sps.frame_num_bits,
                                        sps.max_num_ref_frames,
                                    )?);
                            }
                            let buffer = references.as_mut().unwrap();
                            let lists = buffer.lists(&header, order.before_marking.picture())?;
                            eprintln!("AVC_REFS,{track},{sample},{:?},{:?}", lists.l0, lists.l1);
                            // Metadata simulation only: this inspector does not reconstruct pixels.
                            buffer.finish(
                                &header,
                                order.after_marking.picture(),
                                sample as u64,
                                std::sync::Arc::new(()),
                            )?;
                            eprintln!(
                                "AVC_POC,{track},{sample},{},{}",
                                u8::from(header.idr),
                                order.before_marking.picture()
                            );
                            first_slice = false;
                        }
                        if matches!(
                            header.slice_type,
                            fvid::codec::avc_slice::SliceType::P
                                | fvid::codec::avc_slice::SliceType::B
                        ) && !pps.cabac
                        {
                            let mut reader =
                                fvid::codec::avc_inter_slice::InterCavlcSlice::new_mixed(
                                    &header,
                                    sps,
                                    pps,
                                    16 << 20,
                                )?;
                            let mut counts = [0; 3];
                            let mut inter8 = 0;
                            while let Some(block) = reader.read_macroblock()? {
                                use fvid::codec::avc_inter_slice::InterMacroblock;
                                counts[match block {
                                    InterMacroblock::Skip { .. } => 0,
                                    InterMacroblock::Coded { header, .. } => {
                                        inter8 += usize::from(header.residual.transform8);
                                        1
                                    }
                                    InterMacroblock::Intra(_) => 2,
                                }] += 1;
                            }
                            eprintln!("AVC_INTER_8X8,{inter8}");
                            eprintln!(
                                "AVC_MIXED_MACROBLOCKS,{},{},{}",
                                counts[0], counts[1], counts[2]
                            );
                        }
                        if header.slice_type == fvid::codec::avc_slice::SliceType::I && !pps.cabac {
                            let mut blocks = fvid::codec::avc_macroblock::IntraCavlcReader::new(
                                &header, sps, pps, 65536,
                            )?;
                            let mut count = 0;
                            while blocks.read_macroblock()?.is_some() {
                                count += 1;
                            }
                            eprintln!("AVC_INTRA_MACROBLOCKS,{count}");
                        }
                        eprintln!(
                            "AVC_SLICE,{},{},{:?},{},{},{},{},{}",
                            header.first_mb,
                            header.frame_num,
                            header.slice_type,
                            header.pps_id,
                            header.slice_qp,
                            header.header_bits,
                            header.entropy_bit_offset,
                            header.slice_qp - pps.initial_qp
                        );
                    }
                }
            }
        }
    }
    println!("track,offset,size,dts,pts,duration,sync");
    for (i, track) in mp4.tracks().iter().enumerate() {
        // A track that indexes itself per chunk still holds one frame per
        // sample, so ask for each frame the way playback does.
        for index in 0..track.samples.len() {
            let Some(s) = track.samples.get(index) else {
                break;
            };
            println!(
                "{i},{},{},{},{},{},{}",
                s.offset,
                s.size,
                s.dts,
                s.pts,
                s.duration,
                u8::from(s.sync)
            );
        }
    }
    Ok(())
}
