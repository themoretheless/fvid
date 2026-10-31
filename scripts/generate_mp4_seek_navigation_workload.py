#!/usr/bin/env python3
"""Author a repeatable large packet index for manual CPU seek measurements."""
import sys,json,hashlib
from pathlib import Path
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import mp4
if __name__=='__main__':
 path=Path(sys.argv[1]);count=int(sys.argv[2]) if len(sys.argv)>2 else 50000
 assert 3<=count<=1000000
 seed=generate();packets=[seed[f'frame-{i%3}.packet'] for i in range(count)]
 data=mp4(packets,8,'420',size=(16,16),sync_samples=list(range(1,count+1)));path.write_bytes(data)
 path.with_suffix('.json').write_text(json.dumps({'provenance':'Three authored colored16x16 VP9 packets repeat periodically;50Hz, all sync, no private media, encoder or network. Manual benchmark input, not a regression fixture.','packets':count,'timescale':50,'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest(),'packet_sha256':[hashlib.sha256(seed[f'frame-{i}.packet']).hexdigest() for i in range(3)]},indent=2)+'\n')
