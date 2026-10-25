struct Mp4PacketClockRun {
    first: usize,
    last: usize,
    origin: Mp4ExportTime,
    end: Mp4ExportTime,
    quantized: bool,
    #[allow(dead_code)] // Playback preserves the original grid when no point changed.
    changed: bool,
    rate: u32,
    fixed_frames: u64,
    prefix: Vec<u64>,
}
fn mp4_tick_matches(point: Mp4ExportTime, ticks: i128, scale: u32) -> Result<bool> {
    let lower = point.floor_ticks(scale)?;
    let upper = lower + i128::from(Mp4ExportTime::new(lower,u128::from(scale))? != point);
    Ok(ticks == lower || ticks == upper)
}
fn mp4_packet_clock_runs<R: std::io::Read + std::io::Seek>(
    reader: &mut Mp4TimelineReader<R>, index: usize, control: &DecodeProgress<'_>,
) -> Result<Vec<Mp4PacketClockRun>> {
    let count=reader.tracks()[index].samples.len();
    let count=control.packet_limit().map(|n|usize::try_from(n).unwrap_or(usize::MAX).min(count)).unwrap_or(count);
    let mut result:Vec<Mp4PacketClockRun>=Vec::new();
    let mut first=0;let mut packet=Vec::new();let mut declared_end=None;
    while first<count {
        control.check()?;
        let source=&reader.tracks()[index];
        let sample=source.samples.get(first).ok_or_else(||invalid("missing MP4 clock sample"))?;
        let setup=source.decoder_setup(sample.description_index)?;
        let scale=source.timescale;let rate=setup.sample_rate;
        if scale==0 || rate==0 {return Err(invalid("zero MP4 packet clock"));}
        let mut last=first+1;
        while last<count && source.description_for_sample(last)==source.description_for_sample(first) {last+=1;}
        let declared_origin=Mp4ExportTime::signed_ticks(sample.pts,scale)?;
        let mut origin=declared_origin;
        if let Some(previous)=result.last() {
            if mp4_tick_matches(previous.end,i128::from(sample.pts),scale)? {origin=previous.end;}
        }
        let mut quantized=scale%rate!=0;
        let mut native=0u64;
        let mut changed=false;let mut fixed_frames=None;let mut prefix=Vec::new();
        for i in first..last {
            control.check()?;
            let sample=reader.tracks()[index].samples.get(i).ok_or_else(||invalid("missing MP4 clock sample"))?;
            let begin=i128::from(sample.pts);let end=begin+i128::from(sample.duration);
            if declared_end.is_some_and(|expected|expected!=begin) {return Err(invalid("non-contiguous MP4 audio timeline"));}
            declared_end=Some(end);
            if quantized {
                let payload=if matches!(&setup.codec,b"alac"|b"ac-3") {reader.read_packet(index,i,&mut packet)?;Some(packet.as_slice())}else{None};
                let frames=mp4_coded_packet_frames(&setup.codec,&setup.configuration,rate,setup.channels,setup.bit_depth,sample.size as usize,payload).map_err(|e|invalid(&e))?;
                let start=origin.add(Mp4ExportTime::new(i128::from(native),u128::from(rate))?)?;
                if let Some(fixed)=fixed_frames {
                    if fixed!=frames && prefix.is_empty() {
                        let relative=i-first;
                        prefix.try_reserve_exact(relative.checked_add(2).ok_or_else(||invalid("MP4 clock prefix overflow"))?).map_err(|_|invalid("MP4 clock prefix allocation failed"))?;
                        for n in 0..=relative {prefix.push((n as u64).checked_mul(fixed).ok_or_else(||invalid("MP4 clock prefix overflow"))?);}
                    }
                } else {fixed_frames=Some(frames);}
                native=native.checked_add(frames).ok_or_else(||invalid("MP4 coded clock overflow"))?;
                let finish=origin.add(Mp4ExportTime::new(i128::from(native),u128::from(rate))?)?;
                if !prefix.is_empty() {prefix.try_reserve(1).map_err(|_|invalid("MP4 clock prefix allocation failed"))?;prefix.push(native);}
                changed |= start!=Mp4ExportTime::new(begin,u128::from(scale))? || finish!=Mp4ExportTime::new(end,u128::from(scale))?;
                if !mp4_tick_matches(start,begin,scale)? || !mp4_tick_matches(finish,end,scale)? {quantized=false;}
            }
        }
        let end=if quantized {origin.add(Mp4ExportTime::new(i128::from(native),u128::from(rate))?)?}else{
            origin=declared_origin;
            Mp4ExportTime::new(declared_end.ok_or_else(||invalid("empty MP4 clock run"))?,u128::from(scale))?
        };
        result.try_reserve(1).map_err(|_|invalid("MP4 packet clock allocation failed"))?;
        if !quantized {changed=false;prefix=Vec::new();}
        result.push(Mp4PacketClockRun {first,last,origin,end,quantized,changed,rate,fixed_frames:fixed_frames.unwrap_or(0),prefix});first=last;
    }
    Ok(result)
}

#[allow(dead_code)] // Used by the live playback instantiation of this shared index.
impl Mp4PacketClockRun {
    fn native_before(&self,index:usize)->Result<u64> {
        let relative=index.checked_sub(self.first).filter(|_|index<=self.last).ok_or_else(||invalid("MP4 clock sample outside run"))?;
        if self.prefix.is_empty() {(relative as u64).checked_mul(self.fixed_frames).ok_or_else(||invalid("MP4 clock prefix overflow"))}
        else {self.prefix.get(relative).copied().ok_or_else(||invalid("missing MP4 clock prefix"))}
    }
    fn native_frames(&self,index:usize)->Result<u64> {
        self.native_before(index+1)?.checked_sub(self.native_before(index)?).ok_or_else(||invalid("backwards MP4 coded sample clock"))
    }
    fn point(&self,index:usize)->Result<Mp4ExportTime> {
        self.origin.add(Mp4ExportTime::new(i128::from(self.native_before(index)?),u128::from(self.rate))?)
    }
}
