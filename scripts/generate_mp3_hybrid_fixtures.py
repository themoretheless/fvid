#!/usr/bin/env python3
"""Offline authored hybrid-stage vectors using the shared independent oracle."""
from mp3_synthesis_oracle import oracle
import struct,json,hashlib
from pathlib import Path
R=Path(__file__).resolve().parents[1];D=R/"tests/fixtures/playback-errors/mp3-hybrid";D.mkdir(exist_ok=True)
records=[]
def emit(name,granules):
 raw=struct.pack('>H',len(granules))+b''.join(bytes([kind,mixed])+b''.join(struct.pack('<d',v)for v in spectrum)for kind,mixed,spectrum in granules)
 expected=b''.join(struct.pack('<d',v)for v in oracle(granules))
 (D/f'{name}.case').write_bytes(raw);(D/f'{name}.f64le').write_bytes(expected)
 records.append(dict(name=name,granules=len(granules),sha256=hashlib.sha256(raw).hexdigest(),pcm_sha256=hashlib.sha256(expected).hexdigest()))
positions=sorted(set([18*band for band in range(32)]+[18*band+k for band in [0,1,15,31]for k in range(18)]))
for kind,mixed in [(0,0),(1,0),(2,0),(3,0),(2,2),(2,4),(1,2),(3,2)]:
 for pos in positions:
  source=[0.]*576;source[pos]=1/256
  emit(f'basis-{kind}-{mixed}-{pos}',[(kind,mixed,source),(0,0,[0.]*576),(0,0,[0.]*576)])
for mixed in [0,2,4]:
 granules=[]
 for g,kind in enumerate([0,1,2,2,3,0,0,0]):
  spectrum=[(((g*13+i*7)%29)-14)/4096 if g<6 else 0. for i in range(576)]
  granules.append((kind,mixed if kind else 0,spectrum))
 emit(f'transitions-{mixed}',granules)
(D/'manifest.json').write_text(json.dumps(dict(reference='ISO/IEC 11172-3 clauses 2.4.3.4.10 and Annex A/B',cases=records,video_companion='../g711-alaw-2ch.avi'),indent=2)+'\n')
print(len(records),'authored Layer III hybrid cases')
