#!/usr/bin/env python3
"""Own multiplexed ADTS, independent CRC division and companion videos.
Reads authored fixtures; own region helper only, no foreign codec, network or private media.
"""
import json,subprocess
from generate_adts_crc_fixtures import polynomial
from generate_aac_ssr_fixtures import DEST,field,packed
from generate_he_aac_packet_fixtures import video_fixture

def transport(rows,blob,crc):
    payloads=[blob[r['offset']:r['offset']+r['bytes']] for r in rows];n=len(rows)
    body=bytearray();positions=[]
    for row,payload in zip(rows,payloads):
        if body:positions.append(len(body))
        body+=payload
        if crc and n>1:
            wire=''.join(field(b,8) for b in payload)
            protected=''.join(wire[s['start']:s['end']]+'0'*(s['width']-(s['end']-s['start'])) for s in row['regions'])
            body+=polynomial(protected).to_bytes(2,'big')
    h=bytearray.fromhex(rows[0]['header']);h[1]=(h[1]&~1)|int(not crc);size=7+len(body)+(2*n if crc else 0)
    h[3]=(h[3]&~3)|((size>>11)&3);h[4]=(size>>3)&255;h[5]=(h[5]&31)|((size&7)<<5);h[6]=(h[6]&~3)|(n-1)
    fixed=''.join(field(b,8) for b in h)
    if crc and n>1:
        table=b''.join(v.to_bytes(2,'big') for v in positions);check=polynomial(fixed+''.join(field(b,8) for b in table)).to_bytes(2,'big')
        return bytes(h)+table+check+body
    if crc:
        row=rows[0];wire=''.join(field(b,8) for b in payloads[0]);protected=fixed+''.join(wire[s['start']:s['end']]+'0'*(s['width']-(s['end']-s['start'])) for s in row['regions'])
        return bytes(h)+polynomial(protected).to_bytes(2,'big')+body
    return bytes(h)+body

def main():
    m=json.loads((DEST/'adts-crc.json').read_text());blob=bytearray((DEST/'adts-crc-packets.bin').read_bytes());cases=[];invalid=[]
    profiles=json.loads((DEST/'aac-pce-profile.json').read_text())['cases']
    offset=len(blob);blob.extend((DEST/'aac-pce-profile-packets.bin').read_bytes())
    inputs=[]
    for source in profiles:
        source['name']='pce-windows-'+source['name']
        for row in source['frames']:
            row['offset']+=offset;payload=blob[row['offset']:row['offset']+row['bytes']]
            inputs.append(dict(payload=payload.hex(),asc=source['asc']))
            obj=source['object_type'];size=row['bytes']+9
            header=field(0xfff,12)+'0'+'00'+'0'+field(obj-1,2)+field(6,4)+'0'+'000'+'0000'+field(size,13)+field(0x7ff,11)+'00'
            row['header']=packed(header).hex()
    subprocess.run(['cargo','build','--offline','--quiet','--no-default-features','--example','adts_crc_regions'],capture_output=True,check=True,cwd=DEST.parents[2])
    meta=subprocess.run(['cargo','metadata','--offline','--format-version','1','--no-deps'],capture_output=True,text=True,check=True,cwd=DEST.parents[2])
    from pathlib import Path
    helper=Path(json.loads(meta.stdout)['target_directory'])/'debug/examples/adts_crc_regions'
    result=subprocess.run([str(helper)],input=json.dumps(inputs),capture_output=True,text=True,check=True)
    spans=iter(json.loads(result.stdout))
    for source in profiles:
        for row in source['frames']:row['regions']=next(spans)
    m['cases'].extend(profiles)
    for source in m['cases']:
        for counts in ([2]*6,[3]*4,[4]*3,[1,2,1,4,4]):
            name=source['name']+'-'+('mixed' if len(set(counts))>1 else str(counts[0]));rows=[source['frames'][i%len(source['frames'])] for i in range(12)]
            case=dict(source,name=name,frames=rows,samples=12288,counts=counts,pcm_offset=0)
            case['video']=video_fixture([case],blob,channels=case['channels'],filename='adts-multi-'+name+'-synthetic.mp4')
            case['adts']=[]
            for crc in (False,True):
                at=0;data=bytearray();transport_rows=[]
                for count in counts:
                    frame=transport(rows[at:at+count],blob,crc);transport_rows.append(dict(offset=len(data),bytes=len(frame),blocks=count));data+=frame;at+=count
                assert at==12
                file='adts-multi-'+name+('-crc' if crc else '')+'-synthetic.aac';(DEST/file).write_bytes(data);case['adts'].append(dict(file=file,crc=crc,frames=transport_rows))
            cases.append(case)
    # Each malformed transport retains valid authored AAC; isolate framing/CRC.
    source=cases[0]
    protected=(DEST/source['adts'][1]['file']).read_bytes();plain=(DEST/source['adts'][0]['file']).read_bytes()
    size=source['adts'][1]['frames'][0]['bytes']
    for kind in ['header-crc','raw-crc','position','position-syntax','count']:
        data=bytearray(plain if kind=='count' else protected)
        if kind=='header-crc':data[9]^=1;error='ADTS header CRC mismatch'
        elif kind=='raw-crc':data[size-1]^=1;error='ADTS raw block CRC mismatch'
        elif kind in ('position','position-syntax'):
            value=0 if kind=='position' else int.from_bytes(data[7:9],'big')+1
            data[7:9]=value.to_bytes(2,'big');fixed=''.join(field(b,8) for b in data[:9]);data[9:11]=polynomial(fixed).to_bytes(2,'big')
            error='invalid ADTS raw block position' if kind=='position' else 'ADTS raw block position disagrees with syntax'
        else:data[6]&=~3;data[6]|=2;error='ADTS raw block count disagrees with frame length'
        file='adts-multi-invalid-'+kind+'-synthetic.aac';(DEST/file).write_bytes(data)
        invalid.append(dict(kind=kind,file=file,error=error,valid=source['name']))
    late=bytearray(protected);row=source['adts'][1]['frames'][1];late[row['offset']+row['bytes']-1]^=1
    file='adts-multi-invalid-late-raw-crc-synthetic.aac';(DEST/file).write_bytes(late)
    invalid.append(dict(kind='late-raw-crc',file=file,error='ADTS raw block CRC mismatch',valid=source['name'],warm_blocks=2))
    (DEST/'adts-multi-packets.bin').write_bytes(blob)
    (DEST/'adts-multi.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,oracle='authored block boundaries; independent GF(2) CRC division'),indent=2)+'\n')
    print(len(cases),'own multiblock videos')
if __name__=='__main__':main()
