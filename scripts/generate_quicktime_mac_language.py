#!/usr/bin/env python3
"""Author explicit QuickTime versus ISO packed-language controls."""
import hashlib,json,struct
from pathlib import Path
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/quicktime-mac-language'
def main():
 seed=generate();files={n:b for n,b in seed.items() if n!='three-colors.mp4'};cases=[]
 languages=json.loads((Path(__file__).parent/'data/quicktime-mac-languages.json').read_text())['languages']
 values=[(int(k),v) for k,v in languages.items()]+[(86,''),(95,''),(127,''),(139,''),(1023,''),(0x7fff,''),(0x8000,'')]
 for version in [0,1]:
  for qt in [False,True]:
   for code,language in values:
    def walk(data):
     out=b''
     for k,b in atoms(data):
      if k in [b'moov',b'trak',b'mdia']:b=walk(b)
      elif k==b'ftyp' and qt:b=b'qt  '+word(0)+b'qt  qt  '
      elif k==b'mdhd':
       b=b[:20]+struct.pack('>H',code)+b[22:]
       if version:b=word(1<<24)+bytes(16)+b[12:16]+struct.pack('>Q',int.from_bytes(b[16:20],'big'))+b[20:]
      out+=atom(k,b)
     return out
    n=f'{"qt" if qt else "iso"}-'+('v1-' if version else '')+f'{code}.mp4';files[n]=walk(seed['three-colors.mp4']);cases.append({'file':n,'expected':language if qt else '', 'quicktime':qt,'version':version,'numeric':code})
 def extra(label,data,expected,qt=True):
  files[label+'.mp4']=data;cases.append({'file':label+'.mp4','expected':expected,'quicktime':qt,'kind':'boundary'})
 def replace_ftyp(data,replace):return b''.join(replace(b) if k==b'ftyp' else atom(k,b) for k,b in atoms(data))
 raw=files['qt-0.mp4']
 extra('large-size-ftyp',replace_ftyp(raw,lambda b:struct.pack('>I4sQ',1,b'ftyp',24)+b'qt  '+word(0)),'eng')
 unbranded=replace_ftyp(raw,lambda b:atom(b'free',b))
 extra('unbranded-numeric-zero',unbranded,'',False)
 extra('short-ftyp',unbranded+atom(b'ftyp',b'qt  '),'',False)
 extra('duplicate-ftyp',raw+atom(b'ftyp',b'qt  '+word(0)),'',False)
 extra('compatible-qt-only',replace_ftyp(raw,lambda b:atom(b'ftyp',b'isom'+word(0)+b'qt  isom')),'',False)
 def with_elng(data,payload,duplicate=False):
  out=b''
  for k,b in atoms(data):
   if k in [b'moov',b'trak',b'mdia']:b=with_elng(b,payload,duplicate)
   out+=atom(k,b)
   if k==b'mdhd':
    out+=atom(b'elng',payload)
    if duplicate:out+=atom(b'elng',word(0)+b'de-DE\0')
  return out
 for label,payload,expected,duplicate in [
  ('elng-preferred',word(0)+b'ja-JP\0','ja-JP',False),
  ('elng-und',word(0)+b'und\0','',False),
  ('elng-invalid',word(0)+b'bad_tag\0','rus',False),
  ('elng-version',word(1<<24)+b'ja-JP\0','rus',False),
  ('elng-unterminated',word(0)+b'ja-JP','rus',False),
  ('elng-duplicate',word(0)+b'ja-JP\0','rus',True)]:extra(label,with_elng(files['qt-32.mp4'],payload,duplicate),expected)
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in files.items():(OUT/n).write_bytes(b)
 (OUT/'manifest.json').write_text(json.dumps({'provenance':'Authored three-color16x16 VP9 with explicit QuickTime major brand versus unchanged ISO major brand. Apple Macintosh language0 English and32 Russian; no private media/parameters, encoder or network.','reference':'https://developer.apple.com/documentation/quicktime-file-format/language_code_values','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
