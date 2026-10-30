//! Exact native sample coordinates beneath quantized MP4 packet points.
use super::{Result, invalid, edit_clock::Time as Mp4ExportTime};
use crate::{container::mp4::{Mp4Reader as Mp4TimelineReader, Track},native_media::DecodeProgress};
fn mp4_coded_packet_frames(codec:&[u8;4], configuration:&[u8], rate:u32, channels:u16, bits:u16, bytes:usize, payload:Option<&[u8]>)->std::result::Result<u64,String> {
    mp4_coded_packet_frames_for_programme(0,codec,configuration,rate,channels,bits,bytes,payload)
}
fn mp4_coded_packet_frames_for_programme(programme:u8,codec:&[u8;4], configuration:&[u8], rate:u32, channels:u16, bits:u16, bytes:usize, payload:Option<&[u8]>)->std::result::Result<u64,String> {
    if codec==b"ec-3" {
        let mut encoded=payload.ok_or("E-AC-3 coded count requires packet headers")?;
        let mut samples=0u64;
        while !encoded.is_empty() {
            if let Some(size)=crate::codec::eac3_decoder::skipped_programme_bytes(encoded,programme).map_err(|e|e.to_string())? {
                encoded=&encoded[size..];continue;
            }
            let header=crate::codec::eac3_decoder::programme_frame(encoded,programme).ok_or("invalid selected E-AC-3 coded clock header")?;
            if header.sample_rate!=rate {return Err("selected E-AC-3 coded clock rate changes inside description".into());}
            encoded=encoded.get(header.frame_bytes..).ok_or("incomplete selected E-AC-3 clock packet")?;
            samples=samples.checked_add(header.blocks as u64*256).ok_or("E-AC-3 coded clock overflow")?;
        }
        return Ok(samples);
    }
    if codec!=b"ac-3" {return fvid_media::owned_mp4_audio_packet_frames::frames(codec,configuration,rate,channels,bits,bytes,payload);}
    let mut encoded=payload.ok_or("AC-3 coded count requires packet headers")?;
    let mut frames=0u64;
    while !encoded.is_empty() {
        // CRC/body concealment remains the decoder's decision. The syncframe
        // header bounds each coded access unit; channel geometry is negotiated
        // separately from a CRC-checked header before constructing the decoder.
        let header=crate::codec::ac3_decoder::syncframe(encoded).ok_or("invalid MP4 AC-3 coded clock header")?;
        if header.sample_rate!=rate {return Err("MP4 AC-3 coded clock rate disagrees with description".into());}
        encoded=encoded.get(header.frame_bytes..).ok_or("incomplete MP4 AC-3 clock packet")?;
        frames=frames.checked_add(1536).ok_or("MP4 AC-3 coded clock overflow")?;
    }
    if frames==0 {return Err("empty MP4 AC-3 clock packet".into());}
    Ok(frames)
}
include!("../../../crates/fvid-media/src/owned_mp4_audio_clock_runs_impl.rs");
pub(super) struct PlaybackClock {
    runs: Vec<Mp4PacketClockRun>,
    count: usize,
    scale: u32,
    end: Mp4ExportTime,
}
impl PlaybackClock {
    pub(super) fn build<R:std::io::Read+std::io::Seek>(reader:&mut Mp4TimelineReader<R>, index:usize,programme:u8)->Result<Option<Self>> {
        let track=reader.tracks().get(index).ok_or_else(||invalid("MP4 track index out of range"))?;
        let descriptions=track.sample_descriptions.len();
        let mut rates=Vec::new();
        rates.try_reserve_exact(descriptions).map_err(|_|invalid("MP4 native clock rate allocation failed"))?;
        rates.extend(track.sample_descriptions.iter().map(|d|d.sample_rate));
        let mut coarse=false;
        for i in 0..descriptions {
            let track=&reader.tracks()[index];
            let codec=track.sample_descriptions[i].codec;
            if !(0..track.samples.len()).any(|n|track.samples.get(n).is_some_and(|s|s.description_index as usize==i+1)){continue;}
            if codec==*b"ec-3" {
                if programme>7{return Err(invalid("E-AC-3 programme ID exceeds 7"));}
                let count=track.samples.len();
                let mut payload=Vec::new();let mut found=None;
                for n in 0..count {
                    if reader.tracks()[index].samples.get(n).is_none_or(|s|s.description_index as usize!=i+1){continue;}
                    reader.read_packet(index,n,&mut payload)?;
                    if let Some(frame)=crate::codec::eac3_decoder::checked_programme_frame(&payload,programme) {found=Some(frame.sample_rate);break;}
                }
                rates[i]=found.ok_or_else(||invalid("selected E-AC-3 programme is missing or corrupt in clock setup"))?;
            }
            let track=&reader.tracks()[index];
            if rates[i]==0 || track.timescale==0 {return Err(invalid("zero MP4 playback clock"));}
            coarse |= track.timescale%rates[i]!=0;
        }
        if !coarse {return Ok(None);}
        let control=DecodeProgress::new(None,None)?;
        let override_rates=rates.iter().zip(&reader.tracks()[index].sample_descriptions).any(|(rate,d)|*rate!=d.sample_rate);
        let runs=if programme==0 && !override_rates {mp4_packet_clock_runs(reader,index,&control)?}else{
            mp4_packet_clock_runs_with(reader,index,&control,Some(&rates),|a,b,c,d,e,f,g|mp4_coded_packet_frames_for_programme(programme,a,b,c,d,e,f,g))?
        };
        // A Dolby packet can span an integral number of container ticks while
        // the user's seek target falls between them. Keep its compatible native
        // run for sample-accurate navigation even when no packet point moved.
        let dolby_seek=runs.iter().any(|r|r.quantized && reader.tracks()[index].description_for_sample(r.first).is_some_and(|d|matches!(&d.codec,b"ac-3"|b"ec-3")));
        if !runs.iter().any(|r|r.changed) && !dolby_seek {return Ok(None);}
        let track=&reader.tracks()[index];
        let declared_end=Mp4ExportTime::ticks(track.duration,track.timescale)?;
        let mut end=declared_end;
        if let Some(last)=runs.last() {
            // The recovered endpoint includes its composition-time origin.
            // A decode-duration bound would clip a positively shifted tail.
            end=last.end;
            let decoded_end=(0..track.samples.len()).filter_map(|i|track.samples.get(i))
                .map(|sample|u128::from(sample.dts)+u128::from(sample.duration)).max().unwrap_or(0);
            if u128::from(track.duration)>decoded_end && end.cmp(declared_end)==std::cmp::Ordering::Less {
                end=declared_end;
            }
        }
        let clock=Self{runs,count:track.samples.len(),scale:track.timescale,end};
        for pair in clock.runs.windows(2) {
            if clock.point(track,pair[0].last-1)?.cmp(clock.point(track,pair[1].first)?)==std::cmp::Ordering::Greater {return Err(invalid("non-monotonic recovered MP4 packet clock"));}
        }
        reader.invalidate_position();Ok(Some(clock))
    }
    fn run(&self,index:usize)->Result<&Mp4PacketClockRun> {
        let i=self.runs.partition_point(|r|r.last<=index);
        self.runs.get(i).filter(|r|index>=r.first).ok_or_else(||invalid("MP4 decoder stamp outside source packets"))
    }
    pub(super) fn point(&self,track:&Track,index:usize)->Result<Mp4ExportTime> {
        let run=self.run(index)?;
        if run.quantized {run.point(index)}else{Mp4ExportTime::signed_ticks(track.samples.get(index).ok_or_else(||invalid("missing MP4 clock sample"))?.pts,self.scale)}
    }
    pub(super) fn frames(&self,track:&Track,index:usize)->Result<usize> {
        let run=self.run(index)?;
        let n=if run.quantized {i128::from(run.native_frames(index)?)}else{
            let sample=track.samples.get(index).ok_or_else(||invalid("missing MP4 clock sample"))?;
            let duration=Mp4ExportTime::ticks(u64::from(sample.duration),self.scale)?;let n=duration.ceil_frames(run.rate)?;
            if Mp4ExportTime::new(n,u128::from(run.rate))?!=duration {return Err(invalid("MP4 audio packet clock is not sample aligned"));}n
        };
        usize::try_from(n).map_err(|_|invalid("MP4 decoded window overflow"))
    }
    pub(super) fn full_packet(&self,index:usize)->bool {self.run(index).is_ok_and(|r|r.quantized)}
    pub(super) fn rate(&self,index:usize)->Result<u32>{Ok(self.run(index)?.rate)}
    pub(super) fn end(&self)->Mp4ExportTime {self.end}
    pub(super) fn count(&self)->usize {self.count}
    pub(super) fn at_time(&self,track:&Track,time:Mp4ExportTime)->Result<usize> {
        let (mut a,mut b)=(0,self.count);
        while a<b {let mid=a+(b-a)/2;if self.point(track,mid)?.cmp(time)!=std::cmp::Ordering::Greater {a=mid+1;}else{b=mid;}}
        Ok(a.saturating_sub(1))
    }
    pub(super) fn same_run(&self,a:usize,b:usize)->bool {self.run(a).ok().zip(self.run(b).ok()).is_some_and(|(a,b)|a.first==b.first)}
}
