//! Progressive PCM reconstruction in native component geometry, with 4:4:4 loop filtering.
use super::{IntraPicture, decoded_storage};
use super::super::{avc::{Pps, Sps, SliceGroups}, avc_slice::{SliceHeader,SliceType}, bits::BitReader};
use crate::{Result,invalid,unsupported};
pub(super) fn decode(headers:&[&SliceHeader],sps:&Sps,pps:&Pps,budget:usize)->Result<IntraPicture> {
    if sps.mb_adaptive_frame_field || sps.separate_colour_plane
        || (sps.chroma_format == 3 && !matches!(pps.slice_groups, SliceGroups::Single))
        || headers.iter().any(|h|h.slice_type!=SliceType::I || h.field_pic || h.disable_deblocking_filter_idc>2) {
        return Err(unsupported("AVC non-4:2:0 PCM requires progressive single-group I slices with deblocking disabled"));
    }
    if sps.chroma_format!=3 && (pps.cabac || !matches!(pps.slice_groups, SliceGroups::Single) || headers.iter().any(|h|h.disable_deblocking_filter_idc!=1)) {
        let mut ordered=headers.to_vec();ordered.sort_by_key(|h|h.first_mb);
        return super::decode_component_intra_slices(&ordered,sps,pps,budget);
    }
    let (width,height)=sps.coded_dimensions();let (w,h)=(width as usize,height as usize);
    let (sx,sy)=match sps.chroma_format {0|3=>(1,1),2=>(2,1),_=>return Err(invalid("invalid non-420 AVC PCM format"))};
    if !(8..=14).contains(&sps.bit_depth_luma)||!(8..=14).contains(&sps.bit_depth_chroma) || w==0||h==0||w%16!=0||h%16!=0 {
        return Err(invalid("invalid AVC PCM component geometry or precision"));
    }
    let filter_pcm=sps.chroma_format==3 && headers.iter().any(|h|h.disable_deblocking_filter_idc!=1);
    let filter_work=if filter_pcm {
        (w/16).checked_mul(h/16).and_then(|n|n.checked_mul(std::mem::size_of::<super::super::avc_deblock::MacroblockEdges>()+13))
            .ok_or_else(||invalid("AVC PCM filter storage overflow"))?
    }else{0};
    let needed=decoded_storage(sps,w,h)?.checked_add(filter_work).ok_or_else(||invalid("AVC PCM filter budget overflow"))?;
    if needed>budget {return Err(invalid("AVC PCM picture exceeds memory budget"));}
    let allocate=|count:usize|->Result<Vec<u16>> {
        let mut samples=Vec::new();samples.try_reserve_exact(count).map_err(|_|invalid("cannot allocate AVC PCM plane"))?;
        samples.resize(count,0);Ok(samples)
    };
    let count=w*h;let chroma=if sps.chroma_format==0 {0}else{count/sx/sy};
    let mut picture=IntraPicture {coded_width:w,coded_height:h,crop:sps.crop.map(|v|v as usize),bit_depth:sps.bit_depth_luma,chroma_bit_depth:sps.bit_depth_chroma,
        y:allocate(count)?,cb:allocate(chroma)?,cr:allocate(chroma)?};
    let actual=picture.y.capacity().checked_add(picture.cb.capacity()).and_then(|v|v.checked_add(picture.cr.capacity())).and_then(|v|v.checked_mul(2));
    if actual.and_then(|n|n.checked_add(filter_work)).is_none_or(|v|v>budget) {return Err(invalid("AVC PCM plane capacities exceed budget"));}
    let mut ordered=headers.to_vec();ordered.sort_by_key(|h|h.first_mb);
    let mut seen=0;let mw=w/16;
    for &header in &ordered {
        if header.first_mb as usize!=seen {return Err(invalid("AVC PCM slice coverage gap or overlap"));}
        if pps.cabac {
            let start=seen;
            let mut reader=super::super::avc_cabac::AvcCabac::new(&header.rbsp,header.entropy_bit_offset,SliceType::I,0,header.slice_qp)?;
            loop {
                if seen>=count/256 {return Err(invalid("AVC CABAC PCM macroblock exceeds picture"));}
                let left=seen%mw!=0 && seen>start;
                let top=seen>=mw && seen-mw>=start;
                if !reader.decision(3+usize::from(left)+usize::from(top))? || !reader.terminate()? {
                    drop(picture);drop(reader);
                    return super::intra444::decode(&ordered,sps,pps,budget);
                }
                let (y,[cb,cr])=reader.pcm_raw::<256>(sps.bit_depth_luma,sps.bit_depth_chroma,true)?;
                let bx=seen%mw*16;let by=seen/mw*16;
                for row in 0..16 {
                    for col in 0..16 {
                        let dst=(by+row)*w+bx+col;let src=row*16+col;
                        picture.y[dst]=y[src];picture.cb[dst]=cb[src];picture.cr[dst]=cr[src];
                    }
                }
                seen+=1;
                if reader.terminate()? {reader.finish_slice()?;break;}
            }
            continue;
        }
        let mut bits=BitReader::new(&header.rbsp);bits.skip(header.entropy_bit_offset)?;
        while bits.more_rbsp_data() {
            if seen>=count/256 {return Err(invalid("AVC PCM macroblock exceeds picture"));}
            if bits.unsigned_golomb()?!=25 {
                if sps.chroma_format<=2 {
                    // Reconstruct the complete slice so PCM and compressed
                    // neighbours share prediction and coefficient contexts.
                    // Release the probe planes before allocating that state.
                    drop(picture);
                    return super::decode_component_intra_slices(&ordered,sps,pps,budget);
                }
                drop(picture);
                return super::intra444::decode(&ordered,sps,pps,budget);
            }
            while bits.position()%8!=0 {if bits.bit()? {return Err(invalid("nonzero AVC PCM alignment bit"));}}
            for (component,plane)in [&mut picture.y,&mut picture.cb,&mut picture.cr].into_iter().enumerate() {
                if component!=0 && sps.chroma_format==0 {continue;}
                let (sub_x,sub_y,depth)=if component==0 {(1,1,sps.bit_depth_luma)}else{(sx,sy,sps.bit_depth_chroma)};
                let bw=16/sub_x;let bh=16/sub_y;let stride=w/sub_x;
                let x=seen%(w/16)*bw;let y=seen/(w/16)*bh;
                for row in 0..bh {for col in 0..bw {plane[(y+row)*stride+x+col]=bits.read(depth)?as u16;}}
            }
            seen+=1;
        }
        bits.finish_rbsp()?;
    }
    if seen!=count/256 {return Err(invalid("incomplete AVC PCM picture"));}
    if filter_pcm {
        // PCM macroblocks use QPY=0 for filtering, regardless of slice QP.
        // Full-resolution chroma still derives its component QPC from that 0.
        let mut qps=Vec::new();qps.try_reserve_exact(seen).map_err(|_|invalid("cannot allocate AVC PCM filter QPs"))?;qps.resize(seen,0i32);
        let eight=crate::buffer(seen)?;
        let depths=[sps.bit_depth_luma,sps.bit_depth_chroma,sps.bit_depth_chroma];
        for (p,plane) in [&mut picture.y,&mut picture.cb,&mut picture.cr].into_iter().enumerate() {
            let qp=if p==0 {0}else {
                i32::from(super::chroma_qp(0,if p==1 {pps.chroma_qp_offset}else{pps.second_chroma_qp_offset},depths[p]))-6*(i32::from(depths[p])-8)
            };
            qps.fill(qp);
            super::super::avc_deblock::intra_plane_owned(plane,w,h,&qps,depths[p],&ordered,false,&eight,None)?;
        }
    }
    Ok(picture)
}
