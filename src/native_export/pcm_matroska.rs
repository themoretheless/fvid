//! Bounded float PCM packetization for the owned audio export pipeline.
use crate::{Result, invalid, container::matroska_write::{Encoding, PacketWriter, TrackSpec}};
use std::io::{self, Write, Seek};

pub(super) enum Output<'a,W> {
    Raw(&'a mut W),
    Matroska(Sink<'a,W>),
}
impl<'a,W:Write+Seek> Output<'a,W> {
    pub(super) fn new(output:&'a mut W, matroska:bool, rate:u32, channels:u16)->Result<Self> {
        if matroska {Ok(Self::Matroska(Sink::new(output,rate,channels)?))}else{Ok(Self::Raw(output))}
    }
    pub(super) fn finish(self)->Result<Option<u64>> {
        match self {Self::Raw(_)=>Ok(None),Self::Matroska(sink)=>sink.finish().map(Some)}
    }
}
impl<W:Write+Seek> Write for Output<'_,W> {
    fn write(&mut self,data:&[u8])->io::Result<usize> {
        match self {Self::Raw(output)=>output.write(data),Self::Matroska(sink)=>sink.write(data)}
    }
    fn flush(&mut self)->io::Result<()> {
        match self {Self::Raw(output)=>output.flush(),Self::Matroska(sink)=>sink.flush()}
    }
}
pub(super) struct Sink<'a,W> {
    writer:PacketWriter<'a,W>,
    rate:u32,
    frame_bytes:usize,
    frames:u64,
    bytes:Vec<u8>,
    block_bytes:usize,
}
impl<'a,W:Write+Seek> Sink<'a,W> {
    fn new(output:&'a mut W,rate:u32,channels:u16)->Result<Self> {
        let spec=[TrackSpec {encoding:Encoding::PcmFloat32 {sample_rate:rate,channels},name:"",language:"und"}];
        let writer=PacketWriter::new(output,&spec)?;
        let frame_bytes=usize::from(channels)*4;
        let block_bytes=frame_bytes*1024;
        let mut bytes=Vec::new();bytes.try_reserve_exact(block_bytes).map_err(|_|invalid("cannot allocate PCM packet buffer"))?;
        Ok(Self {writer,rate,frame_bytes,frames:0,bytes,block_bytes})
    }
    fn emit(&mut self)->Result<()> {
        if self.bytes.is_empty() {return Ok(());}
        if self.bytes.len()%self.frame_bytes!=0 {return Err(invalid("incomplete Matroska PCM sample frame"));}
        let next=self.frames.checked_add((self.bytes.len()/self.frame_bytes) as u64).ok_or_else(||invalid("PCM sample clock overflow"))?;
        let time=|frame:u64|u64::try_from(u128::from(frame)*1_000_000_000/u128::from(self.rate)).map_err(|_|invalid("PCM timestamp overflow"));
        let begin=time(self.frames)?;let end=time(next)?;
        self.writer.write_packet(0,begin,end-begin,true,&self.bytes)?;
        self.frames=next;self.bytes.clear();Ok(())
    }
    fn finish(mut self)->Result<u64> {
        self.emit()?;self.writer.finish()?;Ok(self.frames)
    }
}
impl<W:Write+Seek> Write for Sink<'_,W> {
    fn write(&mut self,mut data:&[u8])->io::Result<usize> {
        let count=data.len();
        while !data.is_empty() {
            let take=data.len().min(self.block_bytes-self.bytes.len());
            self.bytes.extend_from_slice(&data[..take]);data=&data[take..];
            if self.bytes.len()==self.block_bytes {self.emit().map_err(io::Error::other)?;}
        }
        Ok(count)
    }
    fn flush(&mut self)->io::Result<()> {self.emit().map_err(io::Error::other)?;self.writer.flush().map_err(io::Error::other)}
}
