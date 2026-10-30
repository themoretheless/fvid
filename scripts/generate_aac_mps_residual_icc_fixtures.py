#!/usr/bin/env python3
"""Owned residual ICC-domain stimuli, separate from tests and C extraction."""
import json
from pathlib import Path
from generate_aac_ld_mps_entropy_fixtures import field,packed,pcm
from generate_aac_ld_mps_config_fixtures import TREES
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-mps-residual-icc'
def main():
 OUT.mkdir(exist_ok=True);blob=bytearray();cases=[]
 for bands in(1,4,23):
  for initial in(False,True):
   for paired in(False,True):
    frames=[];mapped=[0]*bands;coarse=False
    for f,modes in enumerate(([3,3],[1,1],[3,1],[0,0],[0,3],[1,1],[3,3])):
     current=initial if f in(0,6)else not initial;pair=paired and modes==[3,3];wire=''.join(field(m,2)for m in modes);coded={};expected=[];flags=[]
     for ps,mode in enumerate(modes):
      if mode==3 and ps not in coded:
       sets=2 if pair else 1
       rows=[[(b+f+ps+j)%4 if current else [0,1,4,6,7][(b+f+ps+j)%5]for b in range(bands)]for j in range(sets)]
       wire+=field(pair,1)+field(current,1)+'00'+'1'+pcm([rows[j][b]for b in range(bands)for j in range(sets)],4 if current else 8)
       for j,row in enumerate(rows):coded[ps+j]=row
      if mode==0:mapped=[0]*bands;coarse=False
      elif mode==3:mapped=coded[ps][:];coarse=current
      expected.append([v*2 if coarse else v for v in mapped]);flags.append(coarse)
     differences=[[min((f+ps+b)%4,(3 if flags[ps]else 7)-(expected[ps][b]//2 if flags[ps]else expected[ps][b]))for b in range(bands)]for ps in range(2)]
     frames.append(dict(offset=len(blob),bytes=len(packed(wire)),bits=len(wire),independent=f in(0,2,3,4,6),modes=modes,coarse=flags,base=expected,differences=differences));blob.extend(packed(wire))
    cases.append(dict(bands=bands,initial_coarse=initial,paired=paired,frames=frames))
 # Matrix controls retain owned region layouts and CLD/CPC data, with new ICC
 # values/quantization and a declared residual box prefix. Residual audio is
 # supplied separately to the component API; this is not native playback.
 parent=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-assemble';source=json.loads((parent/'manifest.json').read_text());configs=(parent/'configs.bin').read_bytes();matrix_blob=bytearray();matrix_cases=[]
 for base in source['cases']:
  if base['variant']not in(0,1)or base['bands']not in(4,9,23):continue
  for coarse in(False,True):
   for held in(False,True):
    records=[];previous={}
    for f in range(2):
     slots=base['slots'];wire=('11'+field(0,(slots-1).bit_length())+field(slots-2,(slots-1).bit_length())if held else'01')+field(f==0,1);header=wire;expected=[]
     for r in base['frames'][0]['parameters']:
      kind=r['kind'];is_icc=r['id'].startswith('ott-icc:');q=coarse and is_icc;levels,offset={'CLD':(31,15),'ICC':(4 if q else 8,0),'CPC':(51,20)}[kind]
      if f==0:
       rows=[[(b+ps)%4 if q else (b+ps)%5 for b in range(len(r['values'][0]))]for ps in range(2)]if is_icc else r['values'];wire+='1111'+field(True,1)+field(q,1)+'00'+'1'+pcm([rows[ps][b]+offset for b in range(len(rows[0]))for ps in range(2)],levels);previous[r['id']]=rows[-1][:]
      else:wire+='0101';rows=[previous[r['id']][:],previous[r['id']][:]]
      expected.append(dict(id=r['id'],kind=kind,coarse=[q,q],values=[[v*2 if q else v for v in row]for row in rows]))
     corrected={};differences=[]
     for box,lfe in enumerate(TREES[base['tree']][2]):
      if lfe:differences.append(None);continue
      source=next(r for r in expected if r['id']==('ott-icc:all'if base['one_icc']else f'ott-icc:{box}'))
      diff=[min((box+ps+1)%3,(3-source['values'][ps][0]//2)if coarse else(7-source['values'][ps][0]))for ps in range(2)];differences.append(diff)
      values=[row[:]for row in source['values']]
      for ps in range(2):values[ps][0]+=diff[ps]*(2 if coarse else 1)
      corrected[box]=values
     differences += [None]*TREES[base['tree']][3]
     control=header;control_rows=[]
     for r in expected:
      if r['id']=='ott-icc:all':control_rows += [dict(r,id=f'ott-icc:{box}',values=values)for box,values in corrected.items()]
      elif r['id'].startswith('ott-icc:'):control_rows.append(dict(r,values=corrected[int(r['id'].split(':')[1])]))
      else:control_rows.append(r)
     for r in control_rows:
      levels,offset={'CLD':(31,15),'ICC':(8,0),'CPC':(51,20)}[r['kind']]
      rows=r['values'];control+='1111'+'10001'+pcm([rows[ps][b]+offset for b in range(len(rows[0]))for ps in range(2)],levels)
     wire+='0000';packet=packed(wire)+b'\0';control=packed(control+'0000')+b'\0';records.append(dict(offset=len(matrix_blob),bytes=len(packet),parameters=expected,differences=differences,corrected=corrected,control=control.hex()));matrix_blob.extend(packet)
    matrix_cases.append(dict(tree=base['tree'],bands=base['bands'],shared=base['one_icc'],config=configs[base['config_offset']:base['config_offset']+base['config_bytes']].hex(),coarse=coarse,held=held,frames=records))
 (OUT/'matrix-packets.bin').write_bytes(matrix_blob);(OUT/'matrix.json').write_text(json.dumps(dict(scope='Owned external-residual matrix component controls, not compressed residual PCM acceptance.',cases=matrix_cases),separators=(',',':'))+'\n')
 (OUT/'parameters.bin').write_bytes(blob);(OUT/'parameters.json').write_text(json.dumps(dict(cases=cases,scope='Own PCM ecData blocks, raw/coarse metadata and per-set residual ICC differences. No interpolation or out-of-grid reference calls.'),separators=(',',':'))+'\n');print(len(cases),'stateful ICC programmes')
if __name__=='__main__':main()
