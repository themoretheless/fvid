//! Owned single-track AAC MP4 muxing. No decoder or foreign muxer is used.
use crate::{Result, invalid};
use std::io::Write;

fn atom(kind: &[u8; 4], body: &[u8]) -> Result<Vec<u8>> {
    let size = body
        .len()
        .checked_add(8)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| invalid("MP4 atom size overflow"))?;
    Ok([size.to_be_bytes().as_slice(), kind, body].concat())
}
fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
fn table(kind: &[u8; 4], count: u32, entries: &[u8]) -> Result<Vec<u8>> {
    atom(kind, &[&[0; 4][..], &count.to_be_bytes(), entries].concat())
}
fn matrix(bytes: &mut [u8], offset: usize) {
    put(bytes, offset, 0x10000);
    put(bytes, offset + 16, 0x10000);
    put(bytes, offset + 32, 0x40000000);
}

/// Copy ADTS raw AAC packets into an indexed MP4 track. Encoder priming remains
/// unchanged: ADTS provides no authoritative edit information to remove it.
/// Caller owns output and is responsible for discarding partial output on error.
pub fn write_adts_aac(data: &[u8], output: &mut impl Write) -> Result<u64> {
    let stream = super::adts::Aac::parse(data, &Default::default())?;
    let last = stream
        .frames
        .last()
        .ok_or_else(|| invalid("empty ADTS stream"))?;
    if stream.frames[0].start != 0 || last.start + last.size != data.len() {
        return Err(invalid(
            "ADTS remux requires complete frames without unrepresented leading or trailing data",
        ));
    }
    let count =
        u32::try_from(stream.packets()).map_err(|_| invalid("MP4 sample count overflow"))?;
    let duration = u32::try_from(stream.samples()).map_err(|_| invalid("MP4 duration overflow"))?;
    // Version-0 AudioSampleEntry represents sample rate in unsigned 16.16.
    let extended = stream.sample_rate > 65535;
    let ftyp = atom(b"ftyp", if extended { b"qt  \0\0\0\0qt  " } else { b"M4A \0\0\0\0M4A isommp42" })?;
    let sizes: Vec<_> = (0..stream.packets())
        .map(|i| stream.packet(i).len() as u32)
        .collect();
    let payload: u64 = sizes.iter().map(|v| u64::from(*v)).sum();
    let mdat_size = u32::try_from(payload + 8).map_err(|_| invalid("MP4 mdat size overflow"))?;
    let offset = ftyp.len() as u64 + 8;
    let rate = stream.sample_rate;
    let mut mvhd = vec![0; 100];
    put(&mut mvhd, 12, rate);
    put(&mut mvhd, 16, duration);
    put(&mut mvhd, 20, 0x10000);
    mvhd[24..26].copy_from_slice(&0x100u16.to_be_bytes());
    matrix(&mut mvhd, 36);
    put(&mut mvhd, 96, 2);
    let mut tkhd = vec![0; 84];
    put(&mut tkhd, 0, 7);
    put(&mut tkhd, 12, 1);
    put(&mut tkhd, 20, duration);
    tkhd[36..38].copy_from_slice(&0x100u16.to_be_bytes());
    matrix(&mut tkhd, 40);
    let mut mdhd = vec![0; 24];
    put(&mut mdhd, 12, rate);
    put(&mut mdhd, 16, duration);
    mdhd[20..22].copy_from_slice(&0x55c4u16.to_be_bytes()); // und
    let mut hdlr = vec![0; 24];
    hdlr[8..12].copy_from_slice(b"soun");
    hdlr.extend_from_slice(b"FVid Audio\0");
    let mut entry = vec![0; 28];
    entry[6..8].copy_from_slice(&1u16.to_be_bytes());
    entry[16..18].copy_from_slice(&stream.channels.to_be_bytes());
    entry[18..20].copy_from_slice(&16u16.to_be_bytes());
    put(&mut entry, 24, rate << 16);
    if extended {
        // QuickTime v2 stores sample rate as f64 and channels as u32.
        entry.resize(64, 0);
        entry[8..10].copy_from_slice(&2u16.to_be_bytes());
        entry[16..18].copy_from_slice(&3u16.to_be_bytes());
        entry[20..22].copy_from_slice(&(-2i16).to_be_bytes());
        put(&mut entry, 24, 65536);
        put(&mut entry, 28, 72);
        entry[32..40].copy_from_slice(&f64::from(rate).to_be_bytes());
        put(&mut entry, 40, u32::from(stream.channels));
        put(&mut entry, 44, 0x7f000000);
        put(&mut entry, 60, stream.samples_per_frame);
    }
    // Add the terminal SLConfigDescriptor required by the ES descriptor.
    let mut esds = stream.extra_data();
    esds[5] += 3;
    esds.extend_from_slice(&[6, 1, 2]);
    entry.extend(atom(b"esds", &esds)?);
    let stsd = table(b"stsd", 1, &atom(b"mp4a", &entry)?)?;
    let stts = table(
        b"stts",
        1,
        &[count.to_be_bytes(), stream.samples_per_frame.to_be_bytes()].concat(),
    )?;
    let stsc = table(
        b"stsc",
        1,
        &[1u32.to_be_bytes(), count.to_be_bytes(), 1u32.to_be_bytes()].concat(),
    )?;
    let entries: Vec<_> = sizes.iter().flat_map(|size| size.to_be_bytes()).collect();
    let stsz = atom(
        b"stsz",
        &[&[0; 8][..], &count.to_be_bytes(), &entries].concat(),
    )?;
    let co64 = table(b"co64", 1, &offset.to_be_bytes())?;
    let stbl = atom(b"stbl", &[stsd, stts, stsc, stsz, co64].concat())?;
    let dref = table(b"dref", 1, &atom(b"url ", &[0, 0, 0, 1])?)?;
    let dinf = atom(b"dinf", &dref)?;
    let minf = atom(b"minf", &[atom(b"smhd", &[0; 8])?, dinf, stbl].concat())?;
    let mdia = atom(
        b"mdia",
        &[atom(b"mdhd", &mdhd)?, atom(b"hdlr", &hdlr)?, minf].concat(),
    )?;
    let trak = atom(b"trak", &[atom(b"tkhd", &tkhd)?, mdia].concat())?;
    let moov = atom(b"moov", &[atom(b"mvhd", &mvhd)?, trak].concat())?;
    output.write_all(&ftyp)?;
    output.write_all(&mdat_size.to_be_bytes())?;
    output.write_all(b"mdat")?;
    for index in 0..stream.packets() {
        output.write_all(stream.packet(index))?;
    }
    output.write_all(&moov)?;
    Ok(u64::from(count))
}
