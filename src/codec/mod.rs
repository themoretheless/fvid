//! FVid-owned compressed bitstream building blocks.
//! Configuration parsing alone does not imply that a codec can decode frames.
pub use fvid_codecs::codec::*;
// Audio configuration and factory tests keep their original frontend home.
pub mod config;

pub mod aac_decoder;
#[cfg(feature = "player")]
pub mod ac3_decoder;
#[cfg(feature = "player")]
pub mod eac3_decoder;
#[cfg(feature = "player")]
pub mod adpcm_ct_decoder;
#[cfg(feature = "player")]
pub mod adpcm_decoder;
pub mod alac_decoder;
#[cfg(feature = "player")]
pub mod g722_decoder;
#[cfg(feature = "player")]
pub mod g726_decoder;
#[cfg(feature = "player")]
pub mod gsm_decoder;
#[cfg(feature = "player")]
pub mod mace_decoder;
#[cfg(feature = "player")]
pub mod midi_decoder;
pub mod pcm_decoder;
#[cfg(feature = "player")]
pub mod symphonia_decoder;
#[cfg(feature = "player")]
pub mod opus_decoder;
#[cfg(feature = "player")]
pub mod truespeech_decoder;
#[cfg(feature = "player")]
pub mod vorbis_decoder;
#[cfg(feature = "player")]
pub mod wavpack_decoder;
#[cfg(feature = "player")]
pub mod xm_decoder;

/// Build the decoder that matches a container's codec tag.
#[cfg(feature = "player")]
/// `bits_per_sample` is the container's own width for the track, 0 when it
/// declares none; it decides how PCM is read and is ignored by the compressed
/// codecs, which carry the depth in their own setup data.
pub fn make_audio_decoder(
    codec: &str,
    extra_data: &[u8],
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
) -> crate::Result<Box<dyn crate::audio::AudioDecode>> {
    use symphonia::core::codecs::audio::well_known::{
        CODEC_ID_ADPCM_IMA_QT, CODEC_ID_ADPCM_IMA_WAV, CODEC_ID_ADPCM_MS, CODEC_ID_FLAC,
        CODEC_ID_MP2, CODEC_ID_MP3, CODEC_ID_PCM_ALAW, CODEC_ID_PCM_MULAW,
    };
    use symphonia_decoder::SymphoniaDecoder;
    let decoder: Box<dyn crate::audio::AudioDecode> = match codec {
        "mp4a" => Box::new(aac_decoder::AacDecoder::new(
            extra_data,
            sample_rate,
            channels,
        )?),
        "A_OPUS" => Box::new(opus_decoder::OpusDecoder::new(extra_data, sample_rate, channels)?),
        "A_VORBIS" => Box::new(vorbis_decoder::VorbisDecoder::new(
            extra_data,
            sample_rate,
            channels,
        )?),
        // MPEG audio as a bare `.mp3`/`.mp2` file names it, and as Matroska does:
        // the frames describe themselves, so there is no setup block and one
        // symphonia wrapper covers both layers this reader sizes.
        "A_MPEG/L3" => Box::new(SymphoniaDecoder::new(
            "MP3",
            CODEC_ID_MP3,
            extra_data,
            sample_rate,
            channels,
        )?),
        "A_MPEG/L2" => Box::new(SymphoniaDecoder::new(
            "MP2",
            CODEC_ID_MP2,
            extra_data,
            sample_rate,
            channels,
        )?),
        "A_FLAC" => Box::new(SymphoniaDecoder::new(
            "FLAC",
            CODEC_ID_FLAC,
            extra_data,
            sample_rate,
            channels,
        )?),
        // Apple Lossless, named as each container spells it: the fourcc in an ISO
        // BMFF sample entry, the codec ID in a Matroska track. Both put the magic
        // cookie's fields in the setup data, and the coding has no arm in the
        // bundled decoder, so this one is fvid's own.
        "alac" | "A_ALAC" => Box::new(alac_decoder::AlacDecoder::new(
            extra_data,
            sample_rate,
            channels,
        )?),
        // Dolby Digital: Matroska names it by codec ID and an ISO BMFF sample entry
        // by the `ac-3` fourcc. The frames carry their own geometry, so the setup
        // block is empty here, and the bundled decoder has no arm for the coding -
        // this one is fvid's own, transcribed from the public standard.
        "A_AC3" | "ac-3" => Box::new(ac3_decoder::Ac3Decoder::new(
            extra_data,
            sample_rate,
            channels,
        )?),
        // Uncompressed PCM: both containers state the same thing, ISO BMFF in a
        // fourcc per byte order and Matroska in the codec ID, with the width in
        // the sample entry or `BitDepth` - 0 there falls back to 16-bit, which is
        // what a track that says nothing holds.
        "A_PCM/INT/LIT" | "A_PCM/INT/BIG" if bits_per_sample == 8 => Box::new(
            pcm_decoder::PcmDecoder::new(pcm_decoder::PcmFormat::Unsigned8, sample_rate, channels)?),
        "raw " => Box::new(pcm_decoder::PcmDecoder::new(pcm_decoder::PcmFormat::Unsigned8,sample_rate,channels)?),
        "in24" | "in32" => Box::new(pcm_decoder::PcmDecoder::int(
            if codec=="in24" {24} else {32}, extra_data.first()!=Some(&1), sample_rate, channels)?),
        "sowt" | "A_PCM/INT/LIT" => Box::new(pcm_decoder::PcmDecoder::int(
            bits_per_sample,
            false,
            sample_rate,
            channels,
        )?),
        "twos" | "A_PCM/INT/BIG" => Box::new(pcm_decoder::PcmDecoder::int(
            bits_per_sample,
            true,
            sample_rate,
            channels,
        )?),
        // G.711's two companding tables, named as this build's reference names them:
        // a Wave file states them as a format number rather than a tag, and Matroska
        // folds both numbers into `A_MS/ACM` with the setup block carrying it, so
        // neither container hands over a name this dispatch could match on. Each code
        // is one byte wide whatever the track's stated width, so there is no geometry
        // here beyond the rate and the channel count.
        "pcm_alaw" => Box::new(SymphoniaDecoder::new(
            "PCM A-law",
            CODEC_ID_PCM_ALAW,
            extra_data,
            sample_rate,
            channels,
        )?),
        "pcm_mulaw" => Box::new(SymphoniaDecoder::new(
            "PCM Mu-law",
            CODEC_ID_PCM_MULAW,
            extra_data,
            sample_rate,
            channels,
        )?),
        "fl32" | "fl64" | "A_PCM/FLOAT/IEEE" => {
            let mut decoder = pcm_decoder::PcmDecoder::float(
            match codec {
                "fl32" => 32,
                "fl64" => 64,
                _ => bits_per_sample,
            },
            sample_rate,
            channels,
        )?;
            decoder.set_float_big_endian(codec!="A_PCM/FLOAT/IEEE" && extra_data.first()==Some(&0));
            Box::new(decoder)
        },
        // ADPCM as a QuickTime container names it: the `ms\0\xNN` tags carry the
        // WAVE format tag in the last byte (2 for Microsoft's coding, 0x11 for
        // IMA's in a WAV-shaped block) and `ima4` is Apple's own. All three state
        // their geometry in the sample entry rather than the stream, so the setup
        // data decides whether a block can be found at all.
        "adpcm_ms" => Box::new(adpcm_decoder::AdpcmDecoder::new(
            "ADPCM MS",
            CODEC_ID_ADPCM_MS,
            extra_data,
            sample_rate,
            channels,
        )?),
        "adpcm_ima_wav" => Box::new(adpcm_decoder::AdpcmDecoder::new(
            "ADPCM IMA WAV",
            CODEC_ID_ADPCM_IMA_WAV,
            extra_data,
            sample_rate,
            channels,
        )?),
        "adpcm_ima_qt" => Box::new(adpcm_decoder::AdpcmDecoder::new(
            "ADPCM IMA QT",
            CODEC_ID_ADPCM_IMA_QT,
            extra_data,
            sample_rate,
            channels,
        )?),
        // G.722, as this build's reference names the coding. No container spells it
        // with a tag of its own beyond the Wave format number that names it here, and
        // nothing in the stream states a geometry: a code is a byte, its two samples
        // share that byte, and the rate is the container's to say.
        "adpcm_g722" => Box::new(g722_decoder::G722Decoder::new(sample_rate, channels)?),
        // G.726 at any of its four rates: the same dispatch name covers all of them,
        // and which one a stream is comes to the decoder as a code width. A Wave file
        // states that width in its byte rate rather than in the field named for bit
        // depth - measured, the reference ignores the latter entirely - so the reader
        // works it out and hands it over here.
        "adpcm_g726" => Box::new(g726_decoder::G726Decoder::new(
            sample_rate,
            channels,
            u32::from(bits_per_sample),
        )?),
        // GSM 06.10 at its full rate. The rate a Wave track states lives in its block
        // alignment rather than in any width field, and only the whole 65-byte block is
        // decoded, so the reader has already refused the trimmed ones by the time this
        // arm is reached - which is why it takes nothing but the geometry here.
        "gsm_ms" => Box::new(gsm_decoder::GsmDecoder::new(sample_rate, channels)?),
        // Creative's own ADPCM. The Wave tag names it and nothing else about it: a code
        // is four bits, two codes share a byte, and the run has no block, no preamble and
        // no predictor state to align to, so the reader hands over whole bytes cut where
        // its window falls and this arm takes nothing but the geometry.
        "adpcm_ct" => Box::new(adpcm_ct_decoder::AdpcmCtDecoder::new(
            sample_rate,
            channels,
        )?),
        // DSP Group TrueSpeech, as the Wave format number names it. Everything the run
        // needs is in the coding itself - a 32-byte block, 240 samples out of it, eight
        // kHz - and measured the header's width field says nothing about any of it, so
        // this arm reads no `bits_per_sample` and no setup data either.
        "truespeech" => Box::new(truespeech_decoder::TrueSpeechDecoder::new(
            sample_rate,
            channels,
        )?),
        // Apple MACE in either of its two codings, named as this build's reference
        // names them. The fourcc decides which arithmetic a byte's three codes feed,
        // the channel count decides the block's width, and measured neither coding
        // reads anything else from a header: the same bytes decoded the same with the
        // AIFF chunk's sample width stating 1, 8 and 16 bits.
        "mace3" => Box::new(mace_decoder::MaceDecoder::new(
            mace_decoder::MaceCoding::ThreeToOne,
            sample_rate,
            channels,
        )?),
        "mace6" => Box::new(mace_decoder::MaceDecoder::new(
            mace_decoder::MaceCoding::SixToOne,
            sample_rate,
            channels,
        )?),
        // A MIDI performance states no geometry of its own: the container decides
        // the rate and layout it renders at, and hands them over here along with
        // the events as its packets.
        "midi" => Box::new(midi_decoder::MidiDecoder::new(sample_rate, channels)?),
        // A tracker module states its geometry and everything else in the file
        // itself, so the setup data is the module and the packets are its rows.
        "xm" => Box::new(xm_decoder::XmDecoder::new(
            extra_data,
            sample_rate,
            channels,
        )?),
        // WavPack's own lossless coding, spelled the way both this build's raw `.wv`
        // reader and Matroska name it. Everything the decoder needs - the terms, the
        // weights, the depth, the rate - is in the block it is handed, and a block whose
        // own channel count disagrees with the track's is refused rather than reshaped,
        // so this arm takes the geometry only to make the two say the same thing.
        "A_WAVPACK4" | "wavpack" => Box::new(wavpack_decoder::WavpackDecoder::new(channels)?),
        other => return Err(crate::invalid(&format!("unsupported audio codec {other}"))),
    };
    Ok(decoder)
}

pub mod aac_imdct;

pub mod aac_synthesis;

pub mod aac_ics;

pub mod aac_quant;

pub mod aac_pulse;

mod aac_huffman_tables;
pub mod aac_huffman;

pub mod aac_scalefactors;

pub mod aac_spectral;

pub mod aac_bands;

pub mod aac_channel;

pub mod aac_pair;

pub mod aac_noise;

pub mod aac_native;
pub mod aac_pce;

pub mod aac_tns;

pub mod ffv1_encoder;
pub mod ffv1_decoder;





pub mod aac_coupling;
