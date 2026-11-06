//! Determine whether layered dispatch is needed without interpreting VCL data.
use super::*;

pub(super) struct Probe {
    pub may_need_layered: bool,
    pub inband_parameters: bool,
}

pub(super) fn probe<R: Read + Seek>(
    cfg: &HevcConfig,
    configuration: &[u8],
    demux: &mut WebmReader<R>,
    track: u64,
    budget: usize,
) -> Result<Probe> {
    // A configured multilayer candidate keeps the existing activation scan.
    for nal in cfg
        .arrays
        .iter()
        .filter(|a| a.nal_type == 32)
        .flat_map(|a| &a.units)
    {
        if Vps::parse_layered_metadata(nal, budget / 2)?
            .layers
            .effective_max_layers()
            > 1
        {
            return Ok(Probe {
                may_need_layered: true,
                inband_parameters: false,
            });
        }
    }
    let mut parameters = LayeredParameters::from_configuration(configuration, budget / 2)?;
    let mut inband_parameters = false;
    for i in 0..demux.packets.len() {
        if demux.packets[i].track != track {
            continue;
        }
        if demux.packet_payload_size(i)? > budget / 8 {
            return Err(invalid("Matroska HEVC dispatch packet exceeds budget"));
        }
        let packet = demux.read_packet(i)?;
        for nal in crate::codec::config::NalUnits::new(&packet, cfg.length_size)? {
            let Ok(nal) = nal else {
                return Ok(Probe {
                    may_need_layered: false,
                    inband_parameters,
                });
            };
            let h = crate::codec::hevc_nal::NalHeader::parse(nal)?;
            if h.layer_id == 63 {
                continue;
            }
            if h.layer_id != 0 {
                return Ok(Probe {
                    may_need_layered: true,
                    inband_parameters,
                });
            }
            if matches!(h.unit_type, 32..=34) {
                // Retain received-parameter syntax and aggregate bank budgets.
                parameters.receive(nal)?;
                inband_parameters = true;
            }
            if h.unit_type == 32
                && Vps::parse_layered_metadata(nal, budget / 2)?
                    .layers
                    .effective_max_layers()
                    > 1
            {
                return Ok(Probe {
                    may_need_layered: true,
                    inband_parameters,
                });
            }
            // With no multilayer VPS or enhancement NAL, a slice's PPS cannot
            // select a layered operation point. Validate its body when decoded,
            // allowing random access past an independently damaged old picture.
        }
    }
    Ok(Probe {
        may_need_layered: false,
        inband_parameters,
    })
}
