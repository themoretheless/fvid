//! Exact native sample coordinates beneath quantized MP4 packet points.
use super::{Result, invalid, edit_clock::Time as Mp4ExportTime};
use crate::{container::mp4::{Mp4Reader as Mp4TimelineReader, Track},native_media::DecodeProgress};
fn mp4_coded_packet_frames(codec:&[u8;4], configuration:&[u8], rate:u32, channels:u16, bits:u16, bytes:usize, payload:Option<&[u8]>)->std::result::Result<u64,String> {
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
    pub(super) fn build<R:std::io::Read+std::io::Seek>(reader:&mut Mp4TimelineReader<R>, index:usize)->Result<Option<Self>> {
        let track=reader.tracks().get(index).ok_or_else(||invalid("MP4 track index out of range"))?;
        let mut coarse=false;
        for (i,d) in track.sample_descriptions.iter().enumerate() {
            if !(0..track.samples.len()).any(|n|track.samples.get(n).is_some_and(|s|s.description_index as usize==i+1)){continue;}
            if d.codec==*b"ec-3" {return Ok(None);}
            if d.sample_rate==0 || track.timescale==0 {return Err(invalid("zero MP4 playback clock"));}
            coarse |= track.timescale%d.sample_rate!=0;
        }
        if !coarse {return Ok(None);}
        let control=DecodeProgress::new(None,None)?;
        let runs=mp4_packet_clock_runs(reader,index,&control)?;
        // An AC-3 packet can span an integral number of container ticks while
        // the user's seek target falls between them. Keep its compatible native
        // run for sample-accurate navigation even when no packet point moved.
        let ac3_seek=runs.iter().any(|r|r.quantized && reader.tracks()[index].description_for_sample(r.first).is_some_and(|d|d.codec==*b"ac-3"));
        if !runs.iter().any(|r|r.changed) && !ac3_seek {return Ok(None);}
        let track=&reader.tracks()[index];let mut end=Mp4ExportTime::ticks(track.duration,track.timescale)?;
        if track.samples.get(0).is_some_and(|s|s.pts==0) {
            if let Some(last)=runs.last() {
                if last.quantized && mp4_tick_matches(last.end,i128::from(track.duration),track.timescale)? {end=last.end;}
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
