#!/usr/bin/env python3
"""Original implicit SBR ASC with declared 48kHz container output, offline."""
import json,hashlib
from generate_he_aac_packet_fixtures import DEST,packed,field,video_fixture

def main():
    source=json.loads((DEST/'he-aac-sbr-packets.json').read_text());blob=(DEST/'he-aac-sbr-packets.bin').read_bytes()
    cases=[]
    for original in source['cases']:
        if original['bands']!=64 or original['signalling']!='explicit':continue
        case=dict(original);case['signalling']='implicit'
        case['asc']=packed(field(2,5)+field(6,4)+'0001'+field(case['slots']==15,1)+'00').hex()
        cases.append(case)
    video=video_fixture(cases,blob,1,'he-aac-implicit-sbr.mp4')
    case=next(c for c in cases if c['slots']==16)
    def framed(case,raw_blob,channels,filename):
        adts=bytearray()
        for record in case['frames']:
            raw=raw_blob[record['offset']:record['offset']+record['bytes']];length=7+len(raw)
            adts.extend(bytes([0xff,0xf1,0x58,(channels<<6)|((length>>11)&3),(length>>3)&255,((length&7)<<5)|31,0xfc])+raw)
        (DEST/filename).write_bytes(adts)
        return dict(file=filename,sha256=hashlib.sha256(adts).hexdigest(),pcm_offset=case['pcm_offset'],samples=case['samples'])
    adts_info=framed(case,blob,1,'he-aac-implicit-sbr.aac')
    missing=json.loads((DEST/'he-aac-missing-sbr.json').read_text())
    delayed=dict(next(c for c in missing['cases'] if c['slots']==16 and c['bands']==64 and c['pattern']==[False,True,False]))
    delayed.update(asc='1308',signalling='implicit')
    missing_blob=(DEST/'he-aac-missing-sbr.bin').read_bytes()
    delayed_info=dict(video=video_fixture([delayed],missing_blob,1,'he-aac-delayed-sbr.mp4'),
        adts=framed(delayed,missing_blob,1,'he-aac-delayed-sbr.aac'),case=delayed,pcm_file='he-aac-missing-sbr.f64le')
    stereo_source=json.loads((DEST/'he-aac-sbr-stereo.json').read_text())
    stereo=dict(next(c for c in stereo_source['cases'] if c['slots']==16 and c['bands']==64 and c['coupled']))
    stereo.update(asc='1310',signalling='implicit')
    stereo_blob=(DEST/'he-aac-sbr-stereo.bin').read_bytes()
    stereo_info=dict(video=video_fixture([stereo],stereo_blob,2,'he-aac-implicit-sbr-stereo.mp4'),
        adts=framed(stereo,stereo_blob,2,'he-aac-implicit-sbr-stereo.aac'),case=stereo,pcm_file='aac-sbr-dsp-pcm.f64le')
    (DEST/'he-aac-implicit-sbr.json').write_text(json.dumps(dict(kind='original SBR packets with implicit ASC and declared dual-rate output',video=video,adts=adts_info,delayed=delayed_info,stereo=stereo_info,cases=cases),separators=(',',':'))+'\n')
    print(len(cases),'implicit 24-to-48 kHz sequences')
if __name__=='__main__':main()
