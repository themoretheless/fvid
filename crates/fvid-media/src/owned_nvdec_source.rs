//! Codec dispatch over own MP4 inputs and retained native decode tickets.
use crate::{
    owned_mp4::{Limits, Mp4Reader, Track},
    owned_nvdec_avc_decoder::{AvcNvdecDecoder, DecodedAvc},
    owned_nvdec_hevc_decoder::{DecodedHevc, HevcNvdecDecoder},
    owned_nvdec_hevc_mp4::HevcMp4Input,
    owned_nvdec_mp4::{AvcMp4Input, Presentation},
};
use std::io::{Read, Seek};

pub enum MovieSource<R> {
    Avc(AvcMp4Input<R>),
    Hevc(HevcMp4Input<R>),
}
impl<R> From<AvcMp4Input<R>> for MovieSource<R> {
    fn from(source: AvcMp4Input<R>) -> Self {
        Self::Avc(source)
    }
}
impl<R> From<HevcMp4Input<R>> for MovieSource<R> {
    fn from(source: HevcMp4Input<R>) -> Self {
        Self::Hevc(source)
    }
}
#[derive(Clone)]
pub enum DecodedVideo {
    Avc(DecodedAvc),
    Hevc(DecodedHevc),
}
pub enum MovieDecoder {
    Avc(AvcNvdecDecoder),
    Hevc(HevcNvdecDecoder),
}
pub struct MoviePacket {
    pub index: usize,
    pub frame: Option<DecodedVideo>,
}
macro_rules! dispatch {
    ($self:expr,$source:ident,$value:expr) => {
        match $self {
            MovieSource::Avc($source) => $value,
            MovieSource::Hevc($source) => $value,
        }
    };
}
impl<R: Read + Seek> MovieSource<R> {
    pub fn open(input: R, limits: Limits) -> Result<Self, String> {
        let max = limits.packet_bytes;
        let reader = Mp4Reader::open(input, limits).map_err(|e| e.to_string())?;
        let mut videos = reader.tracks().iter().filter(|t| t.handler == *b"vide");
        let codec = videos.next().ok_or("native MP4 has no video")?.codec;
        if videos.next().is_some() {
            return Err("native MP4 requires one video track".into());
        }
        match &codec {
            b"avc1" | b"avc3" => AvcMp4Input::from_reader(reader, max).map(Self::Avc),
            b"hvc1" | b"hev1" => HevcMp4Input::from_reader(reader, max).map(Self::Hevc),
            _ => Err("native MP4 NVDEC currently requires AVC/HEVC".into()),
        }
    }
    pub fn track(&self) -> &Track {
        dispatch!(self, s, s.track())
    }
    pub fn track_count(&self) -> usize {
        dispatch!(self, s, s.track_count())
    }
    pub fn packet_count(&self) -> usize {
        dispatch!(self, s, s.packet_count())
    }
    pub fn coded_dimensions(&self) -> (u32, u32) {
        dispatch!(self, s, s.coded_dimensions())
    }
    pub fn bit_depth(&self) -> u8 {
        match self {
            Self::Avc(_) => 8,
            Self::Hevc(s) => s.bit_depth(),
        }
    }
    pub fn movie_timescale(&self) -> u32 {
        dispatch!(self, s, s.movie_timescale())
    }
    pub fn movie_presentations(&mut self, max: usize) -> Result<Vec<Presentation>, String> {
        match self {
            Self::Avc(s) => s.movie_presentations(max),
            Self::Hevc(s) => s.visible_movie_presentations(max),
        }
    }
    pub fn video_metadata(&self) -> crate::owned_nvdec_movie::MovieVideoMetadata {
        dispatch!(self, s, s.video_metadata())
    }
    pub fn qualify_packets(&mut self) -> Result<(), String> {
        match self {
            Self::Avc(s) => s.qualify_packets(),
            Self::Hevc(s) => s.visible_movie_presentations(1_000_000).map(|_| ()),
        }
    }
    pub fn read_next(
        &mut self,
        bytes: &mut Vec<u8>,
    ) -> Result<Option<crate::owned_mp4::Sample>, String> {
        dispatch!(self, s, s.read_next(bytes))
    }
    pub fn rewind_packets(&mut self) {
        dispatch!(self, s, s.rewind_packets())
    }
    pub fn create_decoder(
        &self,
        ordinal: usize,
        decode: u32,
        output: u32,
    ) -> Result<MovieDecoder, String> {
        match self {
            Self::Avc(s) => s
                .create_decoder(ordinal, decode, output)
                .map(MovieDecoder::Avc),
            Self::Hevc(s) => s
                .create_decoder(ordinal, decode, output)
                .map(MovieDecoder::Hevc),
        }
    }
    pub fn decode_next(
        &mut self,
        decoder: &mut MovieDecoder,
        scratch: &mut Vec<u8>,
    ) -> Result<Option<MoviePacket>, String> {
        match (self, decoder) {
            (Self::Avc(s), MovieDecoder::Avc(d)) => s.decode_next(d, scratch).map(|p| {
                p.map(|p| MoviePacket {
                    index: p.index,
                    frame: Some(DecodedVideo::Avc(p.frame)),
                })
            }),
            (Self::Hevc(s), MovieDecoder::Hevc(d)) => s.decode_next(d, scratch).map(|p| {
                p.map(|p| MoviePacket {
                    index: p.index,
                    frame: p.frame.filter(|f| f.output).map(DecodedVideo::Hevc),
                })
            }),
            _ => Err("native movie source and decoder codecs differ".into()),
        }
    }
}
impl MovieDecoder {
    pub fn map(&mut self, frame: &DecodedVideo) -> Result<fvid_cuda::NvdecSurface, String> {
        match (self, frame) {
            (Self::Avc(d), DecodedVideo::Avc(f)) => d.map(f),
            (Self::Hevc(d), DecodedVideo::Hevc(f)) => d.map(f),
            _ => Err("native movie ticket and decoder codecs differ".into()),
        }
    }
    pub fn unmap(&mut self, slot: usize) -> Result<(), String> {
        match self {
            Self::Avc(d) => d.unmap(slot),
            Self::Hevc(d) => d.unmap(slot),
        }
    }
    pub fn close(&mut self) -> Result<(), String> {
        match self {
            Self::Avc(d) => d.close(),
            Self::Hevc(d) => d.close(),
        }
    }
}
