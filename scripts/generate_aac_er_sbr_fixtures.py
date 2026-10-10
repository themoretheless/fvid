#!/usr/bin/env python3
"""Own ER SBR packets from authored SBR syntax and independent direct-QMF gold."""
import json
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,packet,video_fixture

def main():
    source=json.loads((DEST/'aac-sbr-dsp-oracles.json').read_text())
    syntax=(DEST/'aac-sbr-dsp-syntax.bin').read_bytes();blob=bytearray();cases=[]
    for aot in (17,19):
      for r in source['cases']:
        if r['limiter']!=0 or not r['smoothing']: continue
        slots=r['slots'];rate=48000 if r['bands']==64 else 24000
        for signalling in ('explicit','sync'):
          ga=field(slots==15,1)+'00'+'00'
          asc=packed(field(5,5)+frequency(24000)+'0001'+frequency(rate)+field(aot,5)+ga) if signalling=='explicit' else packed(field(aot,5)+frequency(24000)+'0001'+ga+field(0x2b7,11)+field(5,5)+'1'+frequency(rate))
          rows=[];controls=[];bad=[]
          core='0000'+field(100,8)+'0000'+'000000'+'0'+'000'
          for f in r['frames']:
            raw=syntax[f['offset']:f['offset']+f['byte_length']]
            if raw[0]>>4==14:
              # Frame 2 has identical temporal-zero coefficients without CRC.
              plain=r['frames'][2]
              raw=syntax[plain['offset']:plain['offset']+plain['byte_length']]
            for data,listing in ((packed(core+''.join(field(v,8) for v in raw)),rows),(packet(raw),controls),(packed(core+'1110'+''.join(field(v,8) for v in raw)[4:]),bad)):
              listing.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data)
          c=dict(name=f'{aot}-{slots}-{r["bands"]}-{signalling}',aot=aot,slots=slots,bands=r['bands'],asc=asc.hex(),control_asc=(packed(field(5,5)+frequency(24000)+'0001'+frequency(rate)+field(2,5)+field(slots==15,1)+'00')).hex(),frames=rows,control_frames=controls,bad_frames=bad,pcm_offset=r['pcm_offset'],samples=r['samples'],container_rate=rate,container_frame_samples=slots*r['bands']*2)
          for key,frames,suffix in [('video',rows,''),('bad_video',bad,'-crc-malformed')]:
            c[key]=video_fixture([dict(c,frames=frames)],blob,filename=f'aac-er-sbr-{c["name"]}{suffix}-synthetic.mp4')
          cases.append(c)
    (DEST/'aac-er-sbr-packets.bin').write_bytes(blob)
    (DEST/'aac-er-sbr.json').write_text(json.dumps(dict(cases=cases,provenance='Own silent ER LC/LTP core and own authored SBR noise with temporal envelope history; direct cosine QMF oracle aac-sbr-dsp-pcm.f64le. No private media, foreign encoder/decoder, FFmpeg or network.'),indent=2)+'\n')
    print(len(cases),'ER SBR cases')
if __name__=='__main__':main()
