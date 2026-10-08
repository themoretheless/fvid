#!/usr/bin/env python3
"""Own full PS AAC packet API cases and short malformed MP4 regressions.
Explicit offline generation; only original authored codec/MP4 inputs.
"""
import json,hashlib
from generate_he_aac_packet_fixtures import DEST,asc,packed,video_fixture

def main():
 videos=json.loads((DEST/'aac-ps-matrix-controller-oracles.json').read_text())['videos']+json.loads((DEST/'aac-sbr-ps-30-oracles.json').read_text())['videos']
 cases=[]
 for v in videos:
  slots=v.get('slots',16)
  configs=[dict(signalling=kind,output_rate=rate,asc=asc(24000,rate,slots,kind,ps=True).hex()) for rate in [24000,48000] for kind in ['explicit','sync']]
  cases.append(dict(file=v['video']['file'],slots=slots,configurations=configs))
 v=videos[0];source=(DEST/'he-aac-ps-matrix-controller-packets.bin').read_bytes();original=[source[f['offset']:f['offset']+f['bytes']] for f in v['packet_frames']];raw=bytes.fromhex(v['sbr_payloads'][1]);bits=''.join(f'{b:08b}' for b in original[1]);end=29+3+(4 if len(raw)<15 else 12)+8*len(raw)
 assert bits[end:end+3]=='111'
 malformed=[('trailing-byte',original[1]+b'\xa5','trailing bytes after PS AAC END'),
 ('missing-fill',packed(bits[:29]+'111'),'PS AAC block requires SBR/PS fill'),
 ('duplicate-sce',packed(bits[:29]+bits),'duplicate PS AAC mono element'),
 ('late-element',packed(bits[:end]+'001'+bits[end+3:]),'PS AAC block requires a sole mono SCE'),
 ('fill-before-sce',packed(bits[29:]),'PS SBR fill precedes mono element'),
 ('truncated-fill',original[1][:-1],'truncated AAC fill payload')]
 blob=bytearray();bad=[]
 for name,packet,error in malformed:
  frames=[]
  for p in [original[0],packet,original[2]]:frames.append(dict(offset=len(blob),bytes=len(p)));blob.extend(p)
  case=dict(slots=16,bands=64,frames=frames,asc=v['video']['asc'],pcm_offset=0,samples=12288)
  video=video_fixture([case],blob,channels=2,filename=f'he-aac-ps-native-{name}-synthetic.mp4');video.pop('pcm_offset');video.pop('samples')
  bad.append(dict(name=name,video=video,frames=frames,error=error,failing_packet=1))
 (DEST/'he-aac-ps-native-malformed-packets.bin').write_bytes(blob)
 (DEST/'aac-ps-native-oracles.json').write_text(json.dumps(dict(kind='full native SCE/SBR/PS packet API cases; original synthetic malformed MP4s',cases=cases,malformed=bad,packet_sha256=hashlib.sha256(blob).hexdigest()),indent=2)+'\n')
 print(len(cases),'original native packet cases;',len(bad),'malformed synthetic MP4s')
if __name__=='__main__':main()
