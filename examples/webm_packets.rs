use fvid::container::webm::{Limits, WebmReader};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("expected WebM path")?;
    let mut r = WebmReader::open(
        std::io::BufReader::new(std::fs::File::open(path)?),
        Limits::default(),
    )?;
    // Diagnostics report the whole item, so they take the cost of indexing it.
    r.scan_all()?;
    for track in &r.tracks {
        println!(
            "track {} {} {}x{}",
            track.number, track.codec, track.width, track.height
        );
    }
    println!(
        "packets={} keyframes={} first_pts_ns={:?} last_pts_ns={:?}",
        r.packets.len(),
        r.packets.iter().filter(|p| p.keyframe).count(),
        r.packets.first().map(|p| p.pts_ns),
        r.packets.last().map(|p| p.pts_ns)
    );
    let video_track = r
        .tracks
        .iter()
        .find(|t| t.codec == "V_VP9")
        .map(|t| t.number);
    let times: Vec<_> = r
        .packets
        .iter()
        .filter(|p| Some(p.track) == video_track)
        .map(|p| p.pts_ns)
        .collect();
    println!(
        "non-increasing VP9 packet timestamps={}",
        times.windows(2).filter(|v| v[1] <= v[0]).count()
    );
    let mut headers = fvid::codec::vp9::HeaderState::default();
    let mut coded_frames = 0;
    let mut shown = 0;
    let mut segmented = 0;
    let mut filtered = 0;
    let mut contexts: [fvid::codec::vp9_probs::Probabilities; 4] =
        std::array::from_fn(|_| Default::default());
    let mut tile_count = 0;
    for index in 0..r.packets.len() {
        let p = r.read_packet(index)?;
        if p.is_empty() {
            return Err("empty packet".into());
        }
        if Some(r.packets[index].track) == video_track {
            for frame in fvid::codec::vp9::frames(&p)? {
                let h = headers
                    .parse(frame)
                    .map_err(|e| format!("packet {index}: {e}"))?;
                if h.show_existing.is_none() {
                    let independent = h.is_intra() || h.error_resilient;
                    if independent {
                        if h.keyframe || h.error_resilient || h.reset_context == 3 {
                            contexts = std::array::from_fn(|_| Default::default());
                        } else if h.reset_context == 2 {
                            contexts[usize::from(h.signalled_context)] = Default::default();
                        }
                    }
                    let base = if independent {
                        Default::default()
                    } else {
                        contexts[usize::from(h.context_index())].clone()
                    };
                    let ch = fvid::codec::vp9_probs::CompressedHeader::parse(frame, &h, &base)
                        .map_err(|e| format!("packet {index} compressed header: {e}"))?;
                    if !h.parallel && !h.error_resilient {
                        return Err(format!(
                            "packet {index} needs decoded-symbol probability adaptation"
                        )
                        .into());
                    }
                    if h.refresh_context {
                        contexts[usize::from(h.context_index())] = ch.probabilities;
                    }
                    for tile in fvid::codec::vp9::tiles(frame, &h)? {
                        fvid::codec::vp9_bool::BoolDecoder::new(tile.data)?;
                        tile_count += 1;
                    }
                }
                if coded_frames == 0 {
                    println!("first VP9 header: {h:?}");
                }
                coded_frames += 1;
                shown += usize::from(h.show_frame);
                segmented += usize::from(h.segmentation.enabled);
                filtered += usize::from(h.loop_filter.level > 0);
            }
        }
    }
    println!("all packet payloads read successfully");
    println!("VP9 headers={coded_frames} shown={shown} segmented={segmented} filtered={filtered}");
    println!("VP9 compressed headers parsed, tile markers checked={tile_count}");
    Ok(())
}
