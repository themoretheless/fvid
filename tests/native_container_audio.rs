//! Dolby Digital inside the containers a viewer actually brings.
//!
//! The bare `.ac3` route only ever had to find a syncframe. Here the coding is
//! named by a container instead, and the player has to take that name on faith
//! from a table it did not write: Matroska spells it `A_AC3`, an ISO BMFF sample
//! entry spells it `ac-3`. Both have to reach the same decoder, or the sound a
//! viewer expects is silently missing from the file.
//!
//! These run as an integration target so they exercise only the public surface
//! the player itself uses.

use fvid::audio::AudioStream;
use fvid::codec::make_audio_decoder;
use std::io::Cursor;

/// The 440 Hz sine both fixtures carry, measured through the same decoder the
/// player uses. Returns decoded frames and the loudest sample heard.
fn hear<R: AudioStream + ?Sized>(stream: &mut R) -> (usize, f32, u32, u16) {
    let mut decoder = make_audio_decoder(
        stream.codec(),
        stream.extra_data(),
        stream.sample_rate(),
        stream.channels(),
        stream.bits_per_sample(),
    )
    .expect("the dispatch answers the tag the container gave");
    let rate = stream.sample_rate();
    let channels = stream.channels();
    let bytes = usize::from(channels) * size_of::<f32>();
    let mut frames = 0usize;
    let mut peak = 0.0f32;
    while let Some(packet) = stream.next_packet().expect("packet") {
        let Some(pcm) = decoder
            .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
            .expect("a frame of a real file decodes")
        else {
            continue;
        };
        assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, rate));
        assert_eq!(pcm.data.len() % bytes, 0);
        frames += pcm.data.len() / bytes;
        for sample in pcm.data.chunks_exact(size_of::<f32>()) {
            peak = peak.max(f32::from_le_bytes(sample.try_into().unwrap()).abs());
        }
    }
    (frames, peak, rate, channels)
}

/// The same walk, keeping the loudest sample of every channel apart, so a
/// surround track that quietly drops its rears or its .1 has somewhere to show
/// it.
fn hear_channels<R: AudioStream + ?Sized>(stream: &mut R) -> (usize, Vec<f32>) {
    let mut decoder = make_audio_decoder(
        stream.codec(),
        stream.extra_data(),
        stream.sample_rate(),
        stream.channels(),
        stream.bits_per_sample(),
    )
    .expect("the dispatch answers the tag the container gave");
    let stride = usize::from(stream.channels()) * size_of::<f32>();
    let mut frames = 0usize;
    let mut peaks = vec![0.0f32; stride / size_of::<f32>()];
    while let Some(packet) = stream.next_packet().expect("packet") {
        let Some(pcm) = decoder
            .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
            .expect("a frame of a real file decodes")
        else {
            continue;
        };
        assert_eq!(pcm.data.len() % stride, 0);
        frames += pcm.data.len() / stride;
        for frame in pcm.data.chunks_exact(stride) {
            for (channel, peak) in peaks.iter_mut().enumerate() {
                let offset = channel * size_of::<f32>();
                let sample = frame[offset..offset + size_of::<f32>()]
                    .try_into()
                    .unwrap();
                *peak = (*peak).max(f32::from_le_bytes(sample).abs());
            }
        }
    }
    (frames, peaks)
}

/// Three audio tracks from one source, only one of them Dolby:
///
/// ```sh
/// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=1:s=44100' \
///   -vn -map 0:a -c:a:0 ac3 -b:a:0 96k -map 0:a -c:a:1 flac \
///   -map 0:a -c:a:2 libmp3lame -b:a:2 96k tests/fixtures/audio/ac3-flac-mp3.mkv
/// ```
#[test]
fn a_matroska_file_hands_its_dolby_track_to_the_decoder() {
    const FIXTURE: &[u8] = include_bytes!("fixtures/audio/ac3-flac-mp3.mkv");
    let listing = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(FIXTURE),
        fvid::container::webm::Limits::default(),
    )
    .expect("the file lists audio");
    assert_eq!(
        listing.audio_tracks().len(),
        3,
        "the Dolby track is one of the ones a decoder exists for"
    );

    let mut stream = fvid::playback_webm_audio::WebmAudioReader::open_at(
        Cursor::new(FIXTURE),
        fvid::container::webm::Limits::default(),
        0,
    )
    .expect("the Dolby track opens");
    assert_eq!(stream.codec(), "A_AC3");
    assert_eq!((stream.sample_rate(), stream.channels()), (44_100, 2));
    let (frames, peak, rate, channels) = hear(&mut stream);
    // AC-3 blocks are 1536 samples; a second of sound is a little over 28 of them,
    // and the decoder discards the 256 samples the encoder parked in front.
    assert!(
        (28 * 1_536 - 256..32 * 1_536).contains(&frames),
        "decoded {frames} frames of a one second track"
    );
    assert!((0.25..=0.35).contains(&peak), "peak={peak} of a 0.3 sine");
    assert_eq!((rate, channels), (44_100, 2));
}

/// Half a second of a 440 Hz sine coded as AC-3 and written into an MP4, which
/// is where a camera or a broadcast grab usually keeps it:
///
/// ```sh
/// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=1 \
///   -c:a ac3 -b:a 192k tests/fixtures/audio/ac3-stereo.mp4
/// ```
///
/// The fourcc is `ac-3` and a `dac3` descriptor follows it; ffprobe calls the
/// same stream `ac3`.
#[test]
fn an_mp4_sample_entry_named_ac_dash_3_reaches_the_same_decoder() {
    const FIXTURE: &[u8] = include_bytes!("fixtures/audio/ac3-stereo.mp4");
    let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
        Cursor::new(FIXTURE),
        fvid::container::mp4::Limits::default(),
    )
    .expect("the container indexes the Dolby track");
    assert_eq!(stream.codec(), "ac-3");
    assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 1));
    let (frames, peak, rate, channels) = hear(&mut stream);
    // A whole second at 48 kHz is 31.25 AC-3 blocks; the file is indexed as 32 of
    // them, the same 32 frames ffprobe counts, and each one decodes its full 1536
    // samples whatever fraction of it the track still uses. ffprobe reports this
    // stream's `initial_padding` as 256, and those are the samples the decoder
    // withholds so the sound starts where the writer says it does.
    assert_eq!(
        frames,
        32 * 1_536 - 256,
        "decoded {frames} frames of a one second track"
    );
    assert!(peak > 0.0, "a sine must not come out silent");
    assert_eq!((rate, channels), (48_000, 1));
}

/// Half a second written to six channels, which is the take a viewer reaches for
/// when they say the file is 5.1:
///
/// ```sh
/// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(1100*PI*t)|0.3*sin(1320*PI*t)|0.3*sin(120*2*PI*t)|0.2*sin(440*PI*t)|0.15*sin(110*PI*t):d=0.5:s=48000' \
///   -vn -c:a ac3 -b:a 448k tests/fixtures/audio/ac3-51.mka
/// ```
///
/// Channels here are front left/right, centre, the LFE, then the two rears, so
/// the LFE tone is deliberately low: AC-3 band-limits that channel, and a test
/// that pushed a kilohertz into it would be measuring the low-pass instead of
/// the layout.
#[test]
fn a_matroska_surround_track_reaches_the_decoder_with_six_channels() {
    const FIXTURE: &[u8] = include_bytes!("fixtures/audio/ac3-51.mka");
    let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(FIXTURE),
        fvid::container::webm::Limits::default(),
    )
    .expect("the file lists audio");
    assert_eq!(stream.codec(), "A_AC3");
    assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 6));

    let (frames, peaks) = hear_channels(&mut stream);
    // Sixteen blocks of 1536 samples, minus the 256 the encoder parks in front:
    // the same shift that makes the stereo takes line up with ffprobe.
    assert_eq!(frames, 16 * 1_536 - 256, "decoded {frames} frames");
    assert_eq!(peaks.len(), 6);
    // Each channel comes back at the amplitude the writer put into it, which is
    // also the proof that the six arrived in the order the container says: the
    // last two are the only quiet ones, so a permutation cannot hide.
    let expected = [0.3, 0.3, 0.3, 0.3, 0.2, 0.15];
    for (channel, (peak, want)) in peaks.iter().zip(expected).enumerate() {
        assert!(
            (peak - want).abs() < 0.025,
            "channel {channel} peaked at {peak}, wrote {want}"
        );
    }
}
