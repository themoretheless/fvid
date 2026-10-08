#!/usr/bin/env python3
"""Own delayed PS MP4 export ranges, offline and without external codecs."""
import json,hashlib
from generate_aac_ps_worker_fixtures import edited
from generate_he_aac_packet_fixtures import DEST

def main():
 cases=[]
 for size,reference in [(960,13),(1024,5)]:
  source='he-aac-sbr-ps-960-retain-synthetic.mp4' if size==960 else 'he-aac-ps-matrix-retain-synthetic.mp4'
  name=f'he-aac-ps-export-repeat-{size}-synthetic.mp4'
  edited(source,name,[(1600,-1),(3200,1920),(3200,1920),(1600,-1)])
  cases.append(dict(file=name,reference=reference,source_start=1920,source_samples=3200,silence=1600,presentation_samples=9600,sha256=hashlib.sha256((DEST/name).read_bytes()).hexdigest()))
 (DEST/'aac-ps-mp4-export-oracles.json').write_text(json.dumps(dict(cases=cases),indent=2)+'\n');print('own MP4 PS export repeat fixtures:',len(cases))
if __name__=='__main__':main()
