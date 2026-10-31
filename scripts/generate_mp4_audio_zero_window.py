#!/usr/bin/env python3
"""Authored empty terminal audio windows, separate from rounded native clocks."""
import hashlib
import json
import struct
from pathlib import Path
from generate_mp4_audio_composition_order import generate as short_controls
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom, word
from generate_mp4_quantized_audio_clock import quantize
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/fixtures/playback-errors/mp4-audio-zero-window'


def rewrite(data, timing, scale, duration, codec=None, sizes=None, shift=None, unknown=False, payload=None, payload_tail_bytes=102):
    def walk(body, audio=False):
        out = b''
        for kind, value in atoms(body):
            if kind == b'trak':
                value = walk(value, b'soun' in value)
            elif kind in [b'moov', b'mdia', b'minf', b'stbl']:
                value = walk(value, audio)
                if kind == b'stbl' and audio and shift is not None:
                    count = sum(n for n, _ in timing)
                    value += atom(b'ctts', word(1 << 24, 1, count) + struct.pack('>i', shift))
            elif kind == b'mdat' and payload is not None:
                value = value[:-payload_tail_bytes] + payload
            elif audio and kind == b'stts':
                value = word(0, len(timing)) + b''.join(word(n, d) for n, d in timing)
            elif audio and kind == b'mdhd':
                value = value[:12] + word(scale, 0xffffffff if unknown else duration) + value[20:]
            elif audio and kind == b'tkhd':
                value = value[:20] + word(0 if unknown else (duration * 50 + scale - 1) // scale) + value[24:]
            elif audio and kind == b'stsz' and sizes is not None:
                value = word(0, 0, len(sizes), *sizes)
            elif audio and kind == b'stsd' and codec is not None:
                entry = bytearray(28)
                entry[6:8] = struct.pack('>H', 1)
                entry[16:20] = struct.pack('>HH', 1, 16)
                entry[24:28] = word(8000 << 16)
                value = word(0, 1) + atom(codec, entry)
            out += atom(kind, value)
        return out
    return walk(data)


def generate():
    seed = short_controls()
    original = seed['ordered.mp4']
    pcm = seed['ordered.f32le']
    files = {'ima-control.mp4': original, 'ima-full.f32le': pcm, 'ima-empty.f32le': pcm[:128*4], 'ima-negative.f32le': pcm[64*4:128*4]}
    cases = [{'file': 'ima-control.mp4', 'oracle': 'ima-full.f32le', 'rate': 8000, 'channels': 1, 'shift': 0, 'scale': 8000, 'durations': [64,64,64], 'video_frames': 3, 'reference': 'scalar'}]
    for name, shift, unknown, timing in [
        ('ima-empty', None, False, [(2,64),(1,0)]),
        ('ima-empty-split', None, False, [(1,64),(1,64),(1,0)]),
        ('ima-empty-unknown', None, True, [(2,64),(1,0)]),
        ('ima-empty-positive', 64, False, [(2,64),(1,0)]),
        ('ima-empty-negative', -64, False, [(2,64),(1,0)]),
    ]:
        files[name+'.mp4'] = rewrite(original,timing,8000,128,shift=shift,unknown=unknown)
        cases.append({'file':name+'.mp4','oracle':'ima-negative.f32le' if shift and shift<0 else 'ima-empty.f32le','rate':8000,'channels':1,'shift':shift or 0,'scale':8000,'durations':[64,64,0],'video_frames':3,'reference':'scalar'})
    packets = [seed[f'packet-{i}.ima4'] for i in range(3)]
    files['ima-empty-grouped.mp4'] = rewrite(original,[(2,64),(1,0)],8000,128,sizes=[34,34,68],payload=packets[0]+packets[1]+packets[2]*2)
    cases.append({'file':'ima-empty-grouped.mp4','oracle':'ima-empty.f32le','rate':8000,'channels':1,'shift':0,'scale':8000,'durations':[64,64,0],'video_frames':3,'reference':'scalar'})
    files['ima-bad-two-zero.mp4'] = rewrite(original,[(1,64),(2,0)],8000,64)
    values = [1024,-2048,3072]
    raw = b''.join(struct.pack('<h',v) for v in values)
    files['pcm-full.f32le'] = b''.join(struct.pack('<f',v/32768) for v in values)
    files['pcm-empty.f32le'] = files['pcm-full.f32le'][:8]
    for label, timing, end in [('control',[(3,1)],3),('empty',[(2,1),(1,0)],2)]:
        name='pcm-'+label+'.mp4';files[name]=rewrite(original,timing,8000,end,codec=b'sowt',sizes=[2,2,2],payload=raw)
        cases.append({'file':name,'oracle':'pcm-full.f32le' if label=='control' else 'pcm-empty.f32le','rate':8000,'channels':1,'shift':0,'scale':8000,'durations':[1,1,1] if label=='control' else [1,1,0],'video_frames':3,'reference':'scalar'})
    source=ROOT/'tests/fixtures/playback-errors/mp4-mace-transitions/control-0.mp4'
    files['mace-control.mp4']=source.read_bytes()
    files['mace-empty.mp4']=rewrite(files['mace-control.mp4'],[(234,6),(6,0)],48000,1404)
    for name, empty in [('mace-control.mp4',False),('mace-empty.mp4',True)]:
        cases.append({'file':name,'oracle':None,'rate':8000,'channels':1,'shift':0,'scale':48000,'durations':[36]*39+[0 if empty else 36],'video_frames':6,'reference':'primitive-mace','reference_control':'mace-control.mp4','keep_frames':234 if empty else 240})
    alac=ROOT/'tests/fixtures/playback-errors/mp4-audio-clocks/alac-8000.mp4'
    files['alac-control.mp4']=alac.read_bytes()
    files['alac-full.f32le']=(alac.parent/'expected.f32le').read_bytes()
    files['alac-empty.f32le']=files['alac-full.f32le'][:320*4]
    files['alac-empty.mp4']=rewrite(files['alac-control.mp4'],[(2,160),(1,0)],8000,320)
    for name, empty in [('alac-control.mp4',False),('alac-empty.mp4',True)]:
        cases.append({'file':name,'oracle':'alac-empty.f32le' if empty else 'alac-full.f32le','rate':8000,'channels':1,'shift':0,'scale':8000,'durations':[160,160,0 if empty else 160],'video_frames':3,'reference':'scalar'})
    source=ROOT/'tests/fixtures/playback-errors/mp4-aac-transitions/control-0.mp4'
    files['aac-control.mp4']=source.read_bytes()
    files['aac-full.f32le']=(source.parent/'control-0.f32le').read_bytes()
    files['aac-empty.f32le']=files['aac-full.f32le'][:2048*2*4]
    files['aac-empty.mp4']=rewrite(files['aac-control.mp4'],[(2,301056),(1,0)],14112000,602112)
    top=dict(atoms(files['aac-control.mp4']))
    audio=next(body for kind,body in atoms(top[b'moov']) if kind==b'trak' and b'soun' in body)
    mdia=dict(atoms(audio))[b'mdia'];minf=dict(atoms(mdia))[b'minf'];stbl=dict(atoms(minf))[b'stbl']
    size_table=dict(atoms(stbl))[b'stsz'];sizes=list(struct.unpack('>3I',size_table[12:24]))
    bad_payload=top[b'mdat'][-sum(sizes):-sizes[-1]]+bytes([0])
    files['aac-empty-bad-body.mp4']=rewrite(files['aac-control.mp4'],[(2,301056),(1,0)],14112000,602112,sizes=sizes[:2]+[1],payload=bad_payload,payload_tail_bytes=sum(sizes))
    files['aac-rounded-ceil.mp4']=quantize(files['aac-control.mp4'],25,'ceil')
    for name, oracle, scale, durations in [('aac-control.mp4','aac-full.f32le',14112000,[301056]*3),('aac-empty.mp4','aac-empty.f32le',14112000,[301056,301056,0]),('aac-rounded-ceil.mp4','aac-full.f32le',25,[1,1,0])]:
        cases.append({'file':name,'oracle':oracle,'rate':48000,'channels':2,'shift':0,'scale':scale,'durations':durations,'video_frames':16,'reference':'scalar-aac','rounded_positive_tail':name=='aac-rounded-ceil.mp4'})
    files['manifest.json']=(json.dumps({'provenance':'Existing authored neutral VP9, fresh analytic IMA4/PCM, authored arithmetic MACE and AAC pulse spectra/scalar PCM. No private media/codec parameter sets,encoder or network.','cases':cases,'bad_codec':'aac-empty-bad-body.mp4','malformed_timing':'ima-bad-two-zero.mp4','grouped_budget':'ima-empty-grouped.mp4','donor_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest()for p in [ROOT/'tests/fixtures/playback-errors/mp4-mace-transitions/control-0.mp4',source]},'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n').encode()
    return files


if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
