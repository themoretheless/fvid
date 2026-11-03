"""Author finite-edit packet admission with dormant sample descriptions."""
from pathlib import Path
import json,struct
from fractions import Fraction
from generate_mp4_edit_packet_admission import generate as originals
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-edit-selected-admission'
def generate():
 source=originals();files={};cases=[]
 for codec in ['pcm','alac']:
  for clock,version in [(8000,0),(1001,1)]:
   for edit in ['early','preroll','repeat','gap','silence']:
    original=f'{codec}-clock{clock}-v{version}-{edit}.mp4';base=source[original];oracle=source[original+'.f32le'];files[original]=base;files[original+'.f32le']=oracle
    # Native playback keeps occupied source-sample frontiers across edits.
    # Owned export instead rasterizes cumulative movie boundaries on one grid.
    unit=160 if clock==8000 else 20
    rows={'early':[(unit,0)],'preroll':[(unit,160)],'repeat':[(unit,0),(unit,0)],'gap':[(unit,-1),(unit,0)],'silence':[(unit,-1)]}[edit]
    native=bytearray();cursor=Fraction(0);ticks=0;raw=source[codec+'-full-control.f32le']
    ceil=lambda x:-(-x.numerator//x.denominator)
    for duration,start in rows:
     begin=Fraction(ticks,clock);ticks+=duration;end=Fraction(ticks,clock);first=ceil((max(cursor,begin)-begin)*8000);last=ceil((end-begin)*8000)
     if first<last:
      native+=bytes((last-first)*4)if start<0 else raw[(start+first)*4:(start+last)*4];cursor=begin+Fraction(last,8000)
    files[original+'.native.f32le']=bytes(native)
    for mode in ['actual-first','other-first','opaque-first','duplicate-first']:
     selected=1 if mode=='actual-first'else 2
     def rewrite(data,active=False):
      out=b''
      for k,b in boxes(data):
       if k==b'trak':b=rewrite(b,b'soun'in b)
       elif k in [b'moov',b'mdia',b'minf',b'stbl',b'mvex',b'moof',b'traf']:b=rewrite(b,active)
       elif active and k==b'stsd':
        tag,entry=next(boxes(b[8:]));actual=atom(tag,entry);other=bytearray(28);other[6:8]=b'\0\1';other[16:20]=struct.pack('>HH',2,32);other[24:28]=word(32000<<16);other=atom(b'fl32',other+atom(b'enda',b'\0\1'))
        dormant=atom(b'zzzz',bytes(6)+b'\0\1')if mode=='opaque-first'else actual if mode=='duplicate-first'else other
        b=word(0,2)+(actual+dormant if selected==1 else dormant+actual)
       elif k==b'trex' and int.from_bytes(b[4:8],'big')==2:b=b[:8]+word(selected)+b[12:]
       out+=atom(k,b)
      return out
     name=original.removesuffix('.mp4')+'-'+mode+'.mp4';files[name]=rewrite(base);cases.append(dict(file=name,control=original,oracle=original+'.f32le',native_oracle=original+'.native.f32le',mode=mode,selected=selected,frames=len(oracle)//4,work={'early':1,'preroll':2,'repeat':2,'gap':1,'silence':0}[edit],cap=400))
 for codec in ['pcm','alac']:
  for suffix in ['.mp4','.f32le']:files[codec+'-full-control'+suffix]=source[codec+'-full-control'+suffix]
 files['manifest.json']=(json.dumps(dict(provenance='Separate pure generation from authored VP9/PCM/ALAC regression primitives. Dormant supported/opaque/identical stsd entries; selected fragment description unchanged. Valid larger final packet excluded by finite edits. No private media, encoder or network.',cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
