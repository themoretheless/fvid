"""Author short compact PCM with duplicate stsd catalogs for planning measurements."""
from pathlib import Path
import json,struct
from generate_mp4_uniform_plan_scan import generate as originals
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-admission-catalog'
def generate():
 source=originals()['frames-64.mp4'];parts=dict(boxes(source));count=16384;files={'expected.f32le':struct.pack('<f',0.125)*count};cases=[]
 for catalog,selected in [(1,1),(32,1),(32,32)]:
  def rewrite(data,active=False):
   out=b''
   for k,b in boxes(data):
    if k==b'trak':b=rewrite(b,b'soun'in b)
    elif k in [b'mdia',b'minf',b'stbl']:b=rewrite(b,active)
    elif k==b'mvhd':b=b[:16]+word(18)+b[20:]
    elif active and k==b'tkhd':b=b[:20]+word(18)+b[24:]
    elif active and k==b'mdhd':b=b[:16]+word(count)+b[20:]
    elif active and k==b'stsd':tag,e=next(boxes(b[8:]));b=word(0,catalog)+atom(tag,e)*catalog
    elif active and k==b'stts':b=word(0,1,count,1)
    elif active and k==b'stsc':b=word(0,1,1,count,selected)
    elif active and k==b'stsz':b=word(0,2,count)
    out+=atom(k,b)
   return out
  name=f'catalog{catalog}-selected{selected}.mp4';files[name]=atom(b'ftyp',parts[b'ftyp'])+atom(b'mdat',parts[b'mdat'][:-128]+struct.pack('<h',4096)*count)+atom(b'moov',rewrite(parts[b'moov']));cases.append(dict(file=name,descriptions=catalog,selected=selected,frames=count,rate=48000))
 files['manifest.json']=(json.dumps(dict(provenance='Authored341.334ms mono PCM constant and three neutral VP9 pictures; identical stsd catalogs with one compact PCM chunk. No encoder,private media or network.',cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
