//! Shared first-link audio catalogue across owned Ogg codings.
use crate::owned_ogg::{Coding, Limits, Ogg};
use crate::owned_ogg_opus::AudioTrack;
use fvid_control::error::{Result, invalid};
use std::io::Read;
#[derive(Clone, Debug)]
pub struct Track {
    pub source_index: usize,
    pub codec_ordinal: usize,
    pub coding: Coding,
    pub audio: AudioTrack,
}
pub fn open(mut source: impl Read, limits: Limits) -> Result<Vec<Track>> {
    let mut bytes = Vec::new();
    source
        .by_ref()
        .take((limits.file_bytes as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    let ogg = Ogg::parse(&bytes, &limits)?;
    from_ogg(&ogg)
}
pub fn from_ogg(ogg: &Ogg) -> Result<Vec<Track>> {
    let mut order: Vec<_> = (0..ogg.chains().len()).collect();
    order.sort_by_key(|&i| ogg.chains()[i].first_page);
    let (mut end, mut opus, mut vorbis) = (None, 0, 0);
    let mut tracks = Vec::new();
    for index in order {
        let chain = &ogg.chains()[index];
        if end.is_some_and(|end| chain.first_page > end) {
            break;
        }
        end = Some(end.map_or(chain.last_page, |end: usize| end.max(chain.last_page)));
        let header = &chain
            .packets
            .first()
            .ok_or_else(|| invalid("Ogg stream has no identification"))?
            .data;
        let (rate, channels, ordinal, tags) = match chain.coding {
            Coding::Opus => {
                let head =
                    fvid_opus::OpusHead::parse(header).map_err(|e| invalid(&e.to_string()))?;
                head.decoding_layout()
                    .map_err(|e| invalid(&e.to_string()))?;
                let ordinal = opus;
                opus += 1;
                (
                    48000,
                    u16::from(head.channel_count),
                    ordinal,
                    chain.packets.get(1).map(|p| p.data.clone()),
                )
            }
            Coding::Vorbis => {
                let head = fvid_codecs::vorbis::Identification::parse(header)?;
                let ordinal = vorbis;
                vorbis += 1;
                let tags = chain
                    .packets
                    .get(1)
                    .and_then(|p| p.data.strip_prefix(b"\x03vorbis"))
                    .map(|bytes| {
                        let mut data = b"OpusTags".to_vec();
                        data.extend_from_slice(bytes);
                        data
                    });
                (head.sample_rate, u16::from(head.channels), ordinal, tags)
            }
            _ => continue,
        };
        let tags = tags
            .as_deref()
            .and_then(|t| fvid_opus::OpusTags::parse(t).ok());
        tracks.push(Track {
            source_index: index,
            codec_ordinal: ordinal,
            coding: chain.coding.clone(),
            audio: AudioTrack {
                sample_rate: rate,
                channels,
                name: tags
                    .as_ref()
                    .and_then(|t| t.get("TITLE"))
                    .unwrap_or("")
                    .into(),
                language: tags
                    .as_ref()
                    .and_then(|t| t.get("LANGUAGE"))
                    .unwrap_or("")
                    .into(),
            },
        });
    }
    Ok(tracks)
}

/// Group all overlapping streams into links and follow supported BOS order.
/// Codec changes never remove a link from the selected audio program.
pub fn program_chains(ogg: &Ogg, nth: usize) -> Result<Vec<usize>> {
    let mut order: Vec<_> = (0..ogg.chains().len()).collect();
    order.sort_by_key(|&i| ogg.chains()[i].first_page);
    let mut links: Vec<Vec<usize>> = Vec::new();
    let mut end = 0;
    for index in order {
        let chain = &ogg.chains()[index];
        if links.is_empty() || chain.first_page > end {
            links.push(Vec::new());
            end = chain.last_page;
        } else {
            end = end.max(chain.last_page);
        }
        links.last_mut().unwrap().push(index);
    }
    links
        .into_iter()
        .map(|link| {
            link.into_iter()
                .filter(|&i| matches!(ogg.chains()[i].coding, Coding::Opus | Coding::Vorbis))
                .nth(nth)
                .ok_or_else(|| invalid("selected Ogg audio program is absent in a linked segment"))
        })
        .collect()
}
pub fn needs_joint_program(ogg: &Ogg, nth: usize) -> Result<bool> {
    let chains = program_chains(ogg, nth)?;
    Ok(chains.len() > 1
        && ogg.chains().iter().any(|c| c.coding == Coding::Opus)
        && ogg.chains().iter().any(|c| c.coding == Coding::Vorbis))
}
