#!/usr/bin/env python3
"""Author extended-language metadata around three independently decoded VP9 frames."""
import hashlib,json,struct
from pathlib import Path
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word,ebml
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-extended-language'
def main():
 from generate_rotated_mp4_decode_export_probe_regressions import build_avc_32x16_ipcm,build_single_rotated_mp4,build_rotated_avc_video_track
 avcc,packets=build_avc_32x16_ipcm();base=dict(atoms(build_single_rotated_mp4(avcc,packets)));ftyp=atom(b'ftyp',base[b'ftyp']);mvhd=bytearray(dict(atoms(base[b'moov']))[b'mvhd']);mvhd[12:20]=word(50,2)
 avc=ftyp+atom(b'mdat',b''.join(packets))+atom(b'moov',atom(b'mvhd',mvhd)+build_rotated_avc_video_track(1,avcc,packets,len(ftyp)+8,rotation=0,pasp=None,rate=50,delta=1))
 seed=generate();files={n:b for n,b in seed.items() if n!='three-colors.mp4'};cases=[]
 for i,b in enumerate(packets):files[f'avc-frame-{i}.packet']=b
 def movie(payload,version,duplicate=False,source=None):
  def walk(data):
   out=b''
   for k,b in atoms(data):
    if k in [b'moov',b'trak',b'mdia']:b=walk(b)
    elif k==b'mdhd':
     b=b[:20]+struct.pack('>H',0x15c7)+b[22:]
     if version==1:b=word(1<<24)+bytes(16)+b[12:16]+struct.pack('>Q',int.from_bytes(b[16:20],'big'))+b[20:]
    out+=atom(k,b)
    if k==b'mdhd' and payload is not None:
     out+=atom(b'elng',payload)
     if duplicate:out+=atom(b'elng',word(0)+b'fr-CA\0')
   return out
  return walk(seed['three-colors.mp4'] if source is None else source)
 for v in [0,1]:
  def case(label,payload,expected,kind='valid',duplicate=False):
   n=f'v{v}-{label}.mp4';files[n]=movie(payload,v,duplicate);entry={'file':n,'kind':kind,'expected':expected}
   if kind=='valid':
    entry['copy_file']='avc-'+n;files[entry['copy_file']]=movie(payload,v,source=avc)
   cases.append(entry)
  case('missing',None,'eng','fallback')
  for tag in ['en-AU','fr-CA','zh-Hant','sr-Latn-RS','de-DE-u-co-phonebk','x-fvid-fixture','i-klingon','es-419','zh-cmn-Hans-CN','sl-rozaj-biske-1994','en-a-foobar-x-fvid','EN-au','en','x-a']:
   case('case-'+tag if tag=='EN-au' else tag,word(0)+tag.encode()+b'\0',tag)
  case('en-US',word(0)+b'en-US\0','en-US')
  case('padded-zero',word(0)+b'en-US\0\0\0','en-US')
  case('padded-nontext',word(0)+b'en-US\0\xff\x80','en-US')
  case('trailing-payload',word(0)+b'en-US\0hidden','en-US')
  case('explicit-und',word(0)+b'und\0','','valid')
  long_private='x-'+'-'.join(['part0000']*1024)
  case('long-private-use',word(0)+long_private.encode()+b'\0',long_private)
  long_variants='en-'+'-'.join(f'var{i:05}' for i in range(512))
  case('long-variants',word(0)+long_variants.encode()+b'\0',long_variants)
  boundary='x-'+'-'.join(['part0000']*1820)+'-ab'
  assert len(boundary)==16384
  case('tag-boundary',word(0)+boundary.encode()+b'\0',boundary)
  case('tag-over-limit',word(0)+(boundary+'c').encode()+b'\0',boundary+'c')
  cases[-1]['copy_refusal']=True
  bad=[('empty',word(0)+b'\0'),('missing-terminator',word(0)+b'en-US'),('utf8',word(0)+b'\x80\0'),('underscore',word(0)+b'en_US\0'),('spaces',word(0)+b'en US\0'),('empty-subtag',word(0)+b'en--US\0'),('dangling-extension',word(0)+b'en-u\0'),('reserved-flags',word(1)+b'en-US\0'),('unsupported-version',word(1<<24)+b'en-US\0'),('duplicate-variant',word(0)+b'de-1901-1901\0'),('duplicate-extension',word(0)+b'en-a-foo-a-bar\0'),('case-duplicate-extension',word(0)+b'en-a-foo-A-bar\0'),('case-duplicate-variant',word(0)+b'de-abcde-ABCDE\0'),('numeric-primary',word(0)+b'123\0'),('four-extlangs',word(0)+b'en-aaa-bbb-ccc-ddd\0'),('empty-private',word(0)+b'en-x\0'),('long-subtag',word(0)+b'en-abcdefghi\0'),('punctuation',word(0)+b'en-@\0')]
  for label,payload in bad:case(label,payload,'eng','fallback')
  case('duplicate-atoms',word(0)+b'en-US\0','eng','fallback',True)
 muxed_cases=[]
 parent=OUT.parent/'mp4-compact-sizes'
 files['pcm.f32le']=(parent/'pcm.f32le').read_bytes()
 for kind,original in [('pcm','pcm-stsz-4.mp4'),('text','text-stsz-4-3.mp4')]:
  for version in [0,1]:
   name=f'muxed-{kind}-v{version}.mp4';files[name]=movie(word(0)+b'fr-CA\0\0\0',version,source=(parent/original).read_bytes());muxed_cases.append({'file':name,'kind':kind,'expected':'fr-CA','source':original})
 # Independent standard Matroska witnesses isolate the reader's IETF limit.
 mkv_cases=[]
 for label,tag,refuse in [('normal','fr-CA',False),('long-private',long_private,False),('boundary',boundary,False),('over-limit',boundary+'c',True)]:
  track=ebml('d7',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(20000000).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10'))+ebml('22b59c',b'eng')+ebml('22b59d',tag.encode())
  info=ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',60.0)))
  cluster=ebml('e7',b'\0')+b''.join(ebml('a3',b'\x81'+struct.pack('>h',i*20)+b'\x80'+seed[f'frame-{i}.packet']) for i in range(3))
  header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\2'))
  name=f'ietf-{label}.mkv';files[name]=header+ebml('18538067',info+ebml('1654ae6b',ebml('ae',track))+ebml('1f43b675',cluster));mkv_cases.append({'file':name,'expected':tag,'refusal':refuse})
 assert len({n.casefold() for n in files})==len(files), 'case-insensitive fixture path collision'
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in files.items():(OUT/n).write_bytes(b)
 (OUT/'manifest.json').write_text(json.dumps({'purpose':'Extended language preservation and optional-metadata fallback','provenance':'Authored metadata and existing authored16x16 three-color VP9 packets/oracles; no private data, encoder, FFmpeg or network.','reference':'https://developer.apple.com/documentation/quicktime-file-format/extended_language_tag_atom','cases':cases,'muxed_cases':muxed_cases,'matroska_cases':mkv_cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
