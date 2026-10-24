"""Owned ALAC/ADPCM normalization controls with independent arithmetic PCM."""
from pathlib import Path
from fractions import Fraction as F
import struct,json
from generate_mp4_geometry_normalization import generate as normalization
from generate_mp4_alac_transitions import convert
from generate_mp4_adpcm_transitions import generate as adpcm
from generate_mp4_fragment_description_regressions import atoms
from generate_mp4_variable_rate_edits import edit_movie
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-geometry-lossless-adpcm'
SCALE=14112000
def generate():
    source=normalization();files={};cases=[]
    def third(body,audio=False):
        out=b''
        for tag,b in atoms(body):
            if tag==b'trak':b=third(b,b'soun' in b)
            elif tag in [b'moov',b'mdia',b'minf',b'stbl']:b=third(b,audio)
            elif audio and tag==b'stsd':b=word(0,3)+b''.join(atom(t,v) for t,v in reversed(list(atoms(b[8:]))))
            elif audio and tag==b'stsc':
                b=bytearray(b)
                for i in range(int.from_bytes(b[4:8],'big')):
                    at=16+12*i;b[at:at+4]=word(4-int.from_bytes(b[at:at+4],'big'))
            out+=atom(tag,b)
        return out
    for compressed in [False,True]:
        base=convert(source['pcm.mp4'],compressed)
        for suffix,data,oracle in [('',base,'alac.f32le'),('-third',third(base),'alac.f32le'),('-edits',edit_movie(base,[(1,-1),(180,17),(2,-1),(140,SCALE*30//1000+23)],clock=3000),'alac-edits.f32le')]:
            name=('alac-rice' if compressed else 'alac-raw')+suffix
            files[name+'.mp4']=data;cases.append(dict(name=name,oracle=oracle,rate=48000,channels=2,frames=len(source['pcm-edits.f32le'] if suffix=='-edits' else source['pcm.f32le'])//8,family='alac'))
    files['alac.f32le']=source['pcm.f32le'];files['alac-edits.f32le']=source['pcm-edits.f32le']
    controls=adpcm();manifest=json.loads(controls['manifest.json'])
    for stem in ['regular-2000','fragmented-2000','third-96000','trim-48000','regular-2000-edits']:
        case=next(c for c in manifest['cases'] if c['name']==stem);out=bytearray()
        for part in case['parts']:
            start=F(*part['pts']);duration=F(part['last']-part['first'],part['rate']);begin=-(-(start*8000).numerator//(start*8000).denominator);end=-(-((start+duration)*8000).numerator//((start+duration)*8000).denominator)
            assert len(out)//4==begin
            if part['oracle'] is None:out+=bytes((end-begin)*4)
            elif part['channels']==1 and part['rate']==8000:
                raw=controls[part['oracle']][part['first']*4:part['last']*4];assert len(raw)//4==end-begin;out+=raw
            else:
                raw=controls[part['oracle']][part['first']*8:part['last']*8]
                # Authored stereo predictors are exact opposites at every native
                # sample; their physical mono fold and its filtered output are zero.
                assert all(abs(a+b)<1e-12 for a,b in struct.iter_unpack('<ff',raw))
                out+=bytes((end-begin)*4)
        oracle='adpcm'+('-edits'if stem.endswith('-edits')else'-trim'if stem.startswith('trim')else'')+'.f32le'
        files[oracle]=bytes(out);name='adpcm-'+stem;files[name+'.mp4']=controls[stem+'.mp4'];cases.append(dict(name=name,oracle=oracle,rate=8000,channels=1,frames=len(out)//4,family='adpcm'))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned neutral VP9, arithmetic PCM encoded as ALAC raw/Rice; independently authored IMA4/IMA-WAV/MS-ADPCM predictors and exact cancelled-stereo mono reference. No private media, FFmpeg, external codec/process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
