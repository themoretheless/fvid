#!/usr/bin/env python3
"""Synthetic QuickTime IMA4, IMA WAV and MS ADPCM blocks; no external codecs or source media."""
from generate_pcm_precision_fixtures import ROOT, quicktime_pcm
import struct

if __name__ == '__main__':
    ROOT.mkdir(parents=True, exist_ok=True)
    # Predictor/index zero, low nibble then high nibble both 1: +1 per sample.
    packet = bytes(2) + bytes([0x11]) * 32
    (ROOT / 'ima4-ramp-edits.mov').write_bytes(quicktime_pcm(b'ima4', 16, packet, samples=64))
    fmt = struct.pack('<HHIIHHHH', 17, 2, 48000, 48000 * 16 // 9, 16, 4, 2, 9)
    stereo = bytes(4) + struct.pack('<hBB', -128, 0, 0) + bytes([0x11]) * 4 + bytes(4)
    (ROOT / 'ima-wav-stereo-edits.mov').write_bytes(quicktime_pcm(
        bytes([109, 115, 0, 17]), 16, stereo, samples=9, channels=2, configuration=fmt))
    adaptive = struct.pack('>H', 1024 | 20) + bytes((i * 73 + 19) & 255 for i in range(32))
    adaptive += struct.pack('>H', (65536 - 1024) | 60) + bytes((i * 31 + 237) & 255 for i in range(32))
    (ROOT / 'ima4-adaptive-stereo-edits.mov').write_bytes(quicktime_pcm(
        b'ima4', 16, adaptive, samples=64, channels=2))
    coefficients = [(256,0),(512,-256),(0,0),(192,64),(240,0),(460,-208),(392,-232)]
    for channels, payload, name in [
        (2, bytes([0,0]) + struct.pack('<hhhhhh',16,16,0,-128,-16,-128) + bytes([0x10])*4, 'ms-adpcm-stereo-edits.mov'),
        (1, bytes([3]) + struct.pack('<hhh',16,-3,0) + bytes([0x0f,0x78,0x41,0xe2,0xf0,0x36,0x9a,0x05]), 'ms-adpcm-adaptive-mono-edits.mov'),
    ]:
        frames = 2 + (len(payload)-7*channels)*2//channels
        fmt = struct.pack('<HHIIHHHHH',2,channels,48000,48000*len(payload)//frames,len(payload),4,32,frames,7)
        fmt += b''.join(struct.pack('<hh',a,b) for a,b in coefficients)
        (ROOT / name).write_bytes(quicktime_pcm(bytes([109,115,0,2]),16,payload,
            samples=frames,channels=channels,configuration=fmt))

    # Playback regression: the edit list doubles presentation length while source
    # blocks stay independent. Pair with the authored raw-video companion.
    import hashlib, json
    names=['ima4-ramp-edits.mov','ima-wav-stereo-edits.mov','ms-adpcm-stereo-edits.mov']
    (ROOT/'adpcm-player-edits.json').write_text(json.dumps(dict(
        video_companion='g711-alaw-2ch.avi',
        cases=[dict(file=name,sha256=hashlib.sha256((ROOT/name).read_bytes()).hexdigest(),
                    presentation_frames=frames) for name,frames in zip(names,[128,18,12])]
    ),indent=2)+'\n')

    # Embed independently authored AVC PCM fields into each reproducer, so the
    # same ADPCM edit failure is exercisable in a playable video container.
    from generate_avc_field_pcm_samples import configuration, field
    from avc_fixture_mp4 import mux
    def boxes(data):
        at=0
        while at<len(data):
            size,kind=struct.unpack_from('>I4s',data,at)
            assert size>=8 and at+size<=len(data)
            yield kind,data[at+8:at+size]
            at+=size
    def box(kind,body):return struct.pack('>I4s',len(body)+8,kind)+body
    packets=[]
    for i,bottom in enumerate([False,True]):
        nal=field(bottom,8,i==0)
        packets.append((i,i==0,len(nal).to_bytes(4,'big')+nal))
    video=mux(configuration(8),packets,width=32,height=32,rate=25)
    def shifted_track(data,shift):
        out=b''
        for kind,body in boxes(data):
            if kind in [b'trak',b'mdia',b'minf',b'stbl']:body=shifted_track(body,shift)
            elif kind==b'stco':
                body=bytearray(body)
                for at in range(8,len(body),4):struct.pack_into('>I',body,at,struct.unpack_from('>I',body,at)[0]+shift)
            elif kind==b'tkhd':
                body=bytearray(body);struct.pack_into('>I',body,12,2)
            out+=box(kind,body)
        return out
    video_top=list(boxes(video))
    for name in names:
        audio_top=dict(boxes((ROOT/name).read_bytes()))
        audio_mdat=box(b'mdat',audio_top[b'mdat'])
        video_prefix=b''.join(box(k,v)for k,v in video_top if k!=b'moov')
        track=next(v for k,v in video_top if k==b'moov')
        video_tracks=b''.join(box(k,v)for k,v in boxes(track)if k==b'trak')
        combined=audio_mdat+video_prefix+box(b'moov',audio_top[b'moov']+shifted_track(video_tracks,len(audio_mdat)))
        (ROOT/name.replace('.mov','-video.mov')).write_bytes(combined)
    manifest=json.loads((ROOT/'adpcm-player-edits.json').read_text())
    for item in manifest['cases']:
        item['video_file']=item['file'].replace('.mov','-video.mov')
        item['video_sha256']=hashlib.sha256((ROOT/item['video_file']).read_bytes()).hexdigest()
    (ROOT/'adpcm-player-edits.json').write_text(json.dumps(manifest,indent=2)+'\n')
