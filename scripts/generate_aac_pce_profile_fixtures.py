#!/usr/bin/env python3
"""Own Main/LC/SSR explicit PCE videos and ADTS bootstrap regressions.
Reads only existing authored fixtures; no private source, codec tool or network.
"""
import json,struct,hashlib,subprocess
from generate_aac_ssr_fixtures import DEST,field,frequency,packed,SEQUENCES
from generate_aac_main_tools_fixtures import channel,ics,Filterbank,predict,SEQUENCES as MAIN_SEQUENCES
from generate_aac_main_prediction_fixtures import initial,add
from generate_he_aac_packet_fixtures import video_fixture

def program(prefix,obj,channels,rate=24000,coupling=()):
    # One front mono or pair element, tag 0; optional declared coupling, no mixes or comment.
    bits=prefix+field(0,4)+field(obj-1,2)+frequency(rate)+field(1,4)+field(0,4)+field(0,4)+field(0,2)+field(0,3)+field(len(coupling),4)+'000'+field(channels==2,1)+field(0,4)+''.join(field(independent,1)+field(tag,4) for independent,tag in coupling)
    return bits+'0'*(-len(bits)%8)+field(0,8)
def config(obj,channels,outer=None,rate=24000,outer_rate=None,coupling=()):
    return packed(program(field(obj if outer is None else outer,5)+frequency(rate if outer_rate is None else outer_rate)+'0000'+'000',obj,channels,rate,coupling))
CRC_HELPER=None
def adts(packets,obj,crc=False,rate=24000,configuration=None):
    global CRC_HELPER
    output=bytearray()
    spans=[]
    if crc:
        assert configuration is not None
        rows=[dict(payload=p.hex(),asc=configuration.hex()) for p in packets]
        if CRC_HELPER is None:
            from pathlib import Path
            subprocess.run(['cargo','build','--offline','--quiet','--no-default-features','--example','adts_crc_regions'],capture_output=True,check=True,cwd=DEST.parents[2])
            metadata=subprocess.run(['cargo','metadata','--offline','--format-version','1','--no-deps'],capture_output=True,text=True,check=True,cwd=DEST.parents[2])
            CRC_HELPER=Path(json.loads(metadata.stdout)['target_directory'])/'debug/examples/adts_crc_regions'
        result=subprocess.run([str(CRC_HELPER)],input=json.dumps(rows),text=True,capture_output=True,check=True)
        spans=json.loads(result.stdout)
    for_index=0
    for payload in packets:
        size=len(payload)+(9 if crc else 7)
        header=field(0xfff,12)+'0'+'00'+field(not crc,1)+field(obj-1,2)+frequency(rate)+'0'+'000'+'0000'+field(size,13)+field(0x7ff,11)+'00'
        assert len(header)==56
        if crc:
            from generate_adts_crc_fixtures import polynomial
            raw=''.join(field(b,8) for b in payload)
            protected=header+''.join(raw[s['start']:s['end']]+'0'*(s['width']-(s['end']-s['start'])) for s in spans[for_index])
            check=polynomial(protected).to_bytes(2,'big')
        else:check=b''
        output+=packed(header)+check+payload
        for_index+=1
    return output

def main():
    blob=bytearray();cases=[];invalid=[]
    main=json.loads((DEST/'aac-main-prediction.json').read_text())['cases'][0]
    stereo=json.loads((DEST/'aac-main-tools.json').read_text())['cases'][0]
    ssr=json.loads((DEST/'aac-ssr.json').read_text())['cases']
    for obj in (1,2,3):
        for channels in (1,2):
            name=['','main','lc','ssr'][obj]+'-'+str(channels);packets=[]
            if obj==1:
                source=main if channels==1 else stereo
                data=(DEST/('aac-main-prediction-packets.bin' if channels==1 else 'aac-main-tools-packets.bin')).read_bytes()
                packets=[data[row['offset']:row['offset']+row['bytes']] for row in source['frames']]
                pcm=(DEST/source['pcm_file']).read_bytes()
            elif obj==3:
                source=next(c for c in ssr if c['video']['file']==f'aac-ssr-{channels}-1-1-synthetic.mp4')
                data=(DEST/'aac-ssr-packets.bin').read_bytes();packets=[data[row['offset']:row['offset']+row['bytes']] for row in source['frames']]
                gold=(DEST/'aac-ssr-pcm.f32le').read_bytes();pcm=gold[source['pcm_offset']:source['pcm_offset']+source['pcm_bytes']]
            else:
                banks=[Filterbank() for _ in range(channels)];pcm=bytearray()
                for i,seq in enumerate(SEQUENCES):
                    shape=i%2;info=ics(seq,2,False,shape=shape)
                    wire='0000000' if channels==1 else '0010000'+'1'+info+'00'
                    output=[]
                    for ch in range(channels):
                        count=8 if seq==2 else 1
                        values=[[(-1 if (i+ch+w)%2 else 1)*v for v in [1,-1,1,-1,0,1,-1,0]] for w in range(count)]
                        wire+=channel(seq,[1,1],values,info=info if channels==1 else '')
                        output.append(banks[ch].run(seq,[[float(v*1024) for v in row] for row in values],shape))
                    packets.append(packed(wire+'111'))
                    for row in zip(*output):pcm+=struct.pack('<'+'f'*channels,*row)
            pce_packet=packed(program('101',obj,channels))
            packets=[pce_packet+packets[0]]+packets[1:]
            # Repeat the same configured in-band PCE after startup as well.
            packets[-1]=pce_packet+packets[-1]
            frames=[]
            for packet in packets:frames.append(dict(offset=len(blob),bytes=len(packet),samples=1024));blob.extend(packet)
            case=dict(name=name,object_type=obj,channels=channels,asc=config(obj,channels).hex(),frames=frames,slots=16,bands=32,container_rate=24000,container_frame_samples=1024,pcm_offset=0,samples=len(pcm)//(channels*4))
            assert case['samples']==len(frames)*1024
            case['video']=video_fixture([case],blob,channels=channels,filename='aac-pce-profile-'+name+'-synthetic.mp4')
            case['pcm_file']='aac-pce-profile-'+name+'-pcm.bin';(DEST/case['pcm_file']).write_bytes(pcm)
            case['adts']=[]
            for crc in (False,True):
                file='aac-pce-profile-'+name+('-crc' if crc else '')+'-synthetic.aac';data=adts(packets,obj,crc,configuration=bytes.fromhex(case['asc']));(DEST/file).write_bytes(data)
                case['adts'].append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),crc=crc))
            cases.append(case)
            if obj in (1,3) and channels==1:
                for mismatch in ['profile','rate']:
                    outer=2 if mismatch=='profile' else obj;outer_rate=48000 if mismatch=='rate' else 24000
                    bad=dict(case,name=name+'-'+mismatch,asc=config(obj,channels,outer,outer_rate=outer_rate).hex(),frames=frames[:1],samples=1024)
                    bad['video']=video_fixture([bad],blob,channels=channels,filename='aac-pce-profile-invalid-'+bad['name']+'-synthetic.mp4')
                    file='aac-pce-profile-invalid-'+bad['name']+'-synthetic.aac';data=adts(packets[:1],outer,rate=outer_rate);(DEST/file).write_bytes(data)
                    bad['adts']=dict(file=file,sha256=hashlib.sha256(data).hexdigest());invalid.append(bad)
    for channels in (1,2):
        for point in (0,1,3):
            name=f'main-cce-{channels}-{point}';coupling=[(point==3,1),(point==3,15)]
            pce=packed(program('101',1,channels,coupling=coupling));frames=[];packets=[];pcm=bytearray()
            history={tag:[initial() for _ in range(4)] for tag in (1,15)}
            target_banks=[Filterbank() for _ in range(channels)];source_banks={tag:Filterbank() for tag in (1,15)}
            for frame,seq in enumerate(MAIN_SEQUENCES):
                shape=frame%2;active=frame>=3 and seq!=2;reset=1 if frame==10 else None
                info=ics(seq,1,active,reset,shape);count=8 if seq==2 else 1
                silent=channel(seq,[0],[[0]*4 for _ in range(count)],info=info if channels==1 else '')
                target='0000000'+silent if channels==1 else '0010000'+'1'+info+'00'+silent*2
                sources=[];spectral=[];independent=[]
                for tag in (1,15):
                    source_seq=seq if point!=3 else ([0,0,1,2,2,3,0,0,1,2,3,0] if tag==1 else [0,1,2,2,3,0,0,0,0,0,0,0])[frame]
                    source_shape=shape if tag==1 else 1-shape;windows=8 if source_seq==2 else 1
                    source_active=frame>=3 and source_seq!=2
                    values=[]
                    for w in range(windows):
                        sign=-1 if (frame+w)%2 else 1
                        values.append([sign,-sign,sign,-sign] if tag==1 else [0,sign,-sign,sign])
                    prefix='010'+field(tag,4)+field(point==3,1)+'000'+field(channels==2,1)+field(0,4)+('00' if channels==2 else '')+field(point==1,1)+'0'+'10'
                    source_info=ics(source_seq,1,source_active,reset,source_shape)
                    sources.append(prefix+channel(source_seq,[1],values,info=source_info))
                    reconstructed=[[float(v*1024) for v in row] for row in values]
                    if source_seq==2:history[tag]=[initial() for _ in range(4)]
                    else:reconstructed[0]=predict(history[tag],reconstructed[0],source_active,[1],reset)
                    if point==3:independent.append(source_banks[tag].run(source_seq,reconstructed,source_shape))
                    else:spectral.append(reconstructed)
                # Swap CCE order independently from configured PCE tag order.
                body=target+''.join(sources if frame%2==0 else sources[::-1])
                packet=(pce if frame in (0,11) else b'')+packed(body+'111');packets.append(packet)
                frames.append(dict(offset=len(blob),bytes=len(packet),samples=1024));blob.extend(packet)
                if point==3:
                    mono=[add(a,b) for a,b in zip(*independent)];output=[mono]*channels
                else:
                    combined=[[add(a,b) for a,b in zip(*rows)] for rows in zip(*spectral)]
                    output=[bank.run(seq,combined,shape) for bank in target_banks]
                for row in zip(*output):pcm+=struct.pack('<'+'f'*channels,*row)
            case=dict(name=name,object_type=1,channels=channels,point=point,tags=[1,15],asc=config(1,channels,coupling=coupling).hex(),frames=frames,slots=16,bands=32,container_rate=24000,container_frame_samples=1024,pcm_offset=0,samples=12288)
            case['video']=video_fixture([case],blob,channels=channels,filename='aac-pce-profile-'+name+'-synthetic.mp4')
            case['pcm_file']='aac-pce-profile-'+name+'-pcm.bin';(DEST/case['pcm_file']).write_bytes(pcm);case['adts']=[]
            for crc in (False,True):
                file='aac-pce-profile-'+name+('-crc' if crc else '')+'-synthetic.aac';data=adts(packets,1,crc,configuration=bytes.fromhex(case['asc']));(DEST/file).write_bytes(data)
                case['adts'].append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),crc=crc))
            cases.append(case)
    invalid_packets=[]
    source=next(c for c in cases if c['name']=='main-cce-1-3')
    original=source['frames'][1];payload=blob[original['offset']:original['offset']+original['bytes']-1]
    frame=dict(offset=len(blob),bytes=len(payload),samples=1024);blob.extend(payload)
    bad=dict(source,name='main-cce-truncated-end',frames=[source['frames'][0],frame],samples=2048,error='truncated or oversized bit field')
    bad['video']=video_fixture([bad],blob,channels=1,filename='aac-pce-profile-main-cce-truncated-end-synthetic.mp4')
    packets=[blob[row['offset']:row['offset']+row['bytes']] for row in bad['frames']]
    file='aac-pce-profile-main-cce-truncated-end-synthetic.aac';data=adts(packets,1);(DEST/file).write_bytes(data)
    bad['adts']=dict(file=file,sha256=hashlib.sha256(data).hexdigest());bad['valid_name']=source['name'];invalid_packets.append(bad)
    (DEST/'aac-pce-profile-packets.bin').write_bytes(blob)
    (DEST/'aac-pce-profile.json').write_text(json.dumps(dict(provenance='Own PCE/ADTS bits and authored AAC packet fixtures. Independent existing scalar Main/LC/SSR PCM references. No private media, foreign codec executable or network.',cases=cases,invalid=invalid,invalid_packets=invalid_packets),indent=2)+'\n')
if __name__=='__main__':main()
