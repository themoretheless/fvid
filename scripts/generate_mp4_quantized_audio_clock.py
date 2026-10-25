"""Owned short videos with floor/ceil/nearest coarse audio clocks and independent PCM."""
from pathlib import Path
from fractions import Fraction as F
import struct,json
from generate_mp4_geometry_normalization import generate as geometry
from generate_mp4_geometry_lossless_adpcm import generate as lossless
from generate_mp4_aac_transitions import generate as aac
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-quantized-audio-clock'
def quantize(data,scale,mode,bad=False):
    def q(x):
        x*=scale
        return x.numerator//x.denominator if mode=='floor' else -(-x.numerator//x.denominator) if mode=='ceil' else (2*x.numerator+x.denominator)//(2*x.denominator)
    old=None;durations=[]
    def rewrite(body,audio=False):
        nonlocal old,durations
        out=b''
        for k,b in atoms(body):
            if k==b'traf':
                parts=list(atoms(b));headers=dict(parts)
                if b'tfhd' in headers and int.from_bytes(headers[b'tfhd'][4:8],'big')==2:
                    h=bytearray(headers[b'tfhd']);assert int.from_bytes(h[:4],'big')==0x02003a
                    d=int.from_bytes(h[12:16],'big');t=bytearray(headers[b'tfdt']);assert t[0]==0
                    at=int.from_bytes(t[4:8],'big');h[12:16]=word(q(F(at+d,old))-q(F(at,old)));t[4:8]=word(q(F(at,old)))
                    b=b''.join(atom(tg,h if tg==b'tfhd' else t if tg==b'tfdt' else v)for tg,v in parts)
            elif k==b'trak':
                selected=b'soun'in b
                if selected:
                    def find_scale(body):
                        for t,v in atoms(body):
                            if t==b'mdhd':return int.from_bytes(v[12:16],'big')
                            if t==b'mdia':return find_scale(v)
                    old=find_scale(b)
                b=rewrite(b,selected)
            elif k in [b'moov',b'mdia',b'minf',b'stbl',b'edts',b'moof']:b=rewrite(b,audio)
            elif audio and k==b'mdhd':
                old=int.from_bytes(b[12:16],'big');duration=int.from_bytes(b[16:20],'big');b=bytearray(b);b[12:20]=word(scale,q(F(duration,old)))
            elif audio and k==b'elst':
                b=bytearray(b)
                for at in range(8,len(b),12):
                    media=struct.unpack('>i',b[at+4:at+8])[0]
                    if media>=0:b[at+4:at+8]=struct.pack('>i',q(F(media,old)))
            elif audio and k==b'stts':
                assert old is not None
                at=F(0);last=0
                for count,d in struct.iter_unpack('>II',b[8:]):
                    for _ in range(count):at+=F(d,old);end=q(at);durations.append(end-last);last=end
                if bad:durations[-1]+=1
                b=word(0,len(durations))+b''.join(word(1,d)for d in durations)
            out+=atom(k,b)
        return out
    result=rewrite(data)
    if bad:
        # The declared sample table deliberately ends beyond the exact 64 ms
        # coded endpoint. This is not an adjacent-tick rounding of that point.
        pass
    return result

def generate(playback_controls=False):
    g=geometry();l=lossless();a=aac();files={};cases=[]
    sources=[('pcm',g['pcm.mp4'],g['pcm.f32le'],48000),('aac',g['aac-changes.mp4'],g['aac.f32le'],48000),('ssr',g['ssr-changes.mp4'],g['ssr.f32le'],24000),('alac-raw',l['alac-raw.mp4'],l['alac.f32le'],48000),('alac-rice',l['alac-rice.mp4'],l['alac.f32le'],48000),('constant-aac',a['control-0.mp4'],a['control-0.f32le'],48000)]
    sources += [('aac-edits',g['aac-changes-edits.mp4'],g['aac-edits.f32le'],48000),('ssr-edits',g['ssr-changes-edits.mp4'],g['ssr-edits.f32le'],24000),('pcm64',g['pcm64.mp4'],g['pcm64.f32le'],48000),('adpcm',l['adpcm-regular-2000.mp4'],l['adpcm.f32le'],8000),('opaque',g['opaque-same-geometry.mp4'],g['opaque-same-geometry.f32le'],48000)]
    sources += [(name,g[name+'.mp4'],g[oracle],rate)for name,oracle,rate in [('pcm-fragmented','pcm.f32le',48000),('aac-fragmented','aac.f32le',48000),('ssr-fragmented','ssr.f32le',24000)]]
    fixture_root=OUT.parent
    references=json.loads((fixture_root/'aac-ps-dsp-oracles.json').read_text());raw=(fixture_root/'aac-ps-dsp-reference.bin').read_bytes()
    source_pcm=[]
    for frame in references['cases'][5]['frames']:
        for i in range(frame['Double'][0][1]):
            for channel in range(2):
                at=frame['Double'][channel][0]+i*8;source_pcm.append(struct.unpack('<d',raw[at:at+8])[0]/32768.)
    expected=bytes(1600*8)+b''.join(struct.pack('<f',v)for v in source_pcm[1920*2:5120*2])*2+bytes(1600*8)
    sources.append(('ps-edits',(fixture_root/'he-aac-ps-inband-export-lc-1024-synthetic.mp4').read_bytes(),expected,48000))
    for name,source,pcm,rate in sources:
        if playback_controls:files['exact-'+name+'.mp4']=source
        files[name+'.f32le']=pcm
        channels=1 if name=='adpcm' else 4 if name=='opaque' else 2
        for scale in [900,1000]:
            for mode in ['floor','ceil','nearest']:
                stem=f'{name}-{scale}-{mode}';files[stem+'.mp4']=quantize(source,scale,mode);cases.append(dict(name=stem,oracle=name+'.f32le',rate=rate,channels=channels,frames=len(pcm)//(channels*4)))
    files['pcm64.f64le']=g['pcm64.f64le']
    files['false-exact-end-gap.mp4']=quantize(a['control-0.mp4'],1000,'floor',True)
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='FVid-owned neutral VP9, arithmetic PCM/ALAC, authored AAC/SSR spectra and independent scalar PCM. Coarse clocks quantize cumulative endpoints, never each duration independently. No private media, FFmpeg, network or subprocess.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
