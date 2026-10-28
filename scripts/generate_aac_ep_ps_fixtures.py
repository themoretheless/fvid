"""Owned EP2 protected ER PS, delayed packet identity and EOF fixtures."""
import contextlib,copy,io,json,tempfile
from pathlib import Path
import generate_aac_er_ps_fixtures as source_generator
from generate_aac_ep_concatenated_fixtures import config,protect
from generate_aac_eld_fixtures import mkv
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-ep-ps'

def originals(frames=14,include_bits=True):
    original_dest=source_generator.DEST;original_video=source_generator.video_fixture
    with tempfile.TemporaryDirectory(prefix='fvid-own-er-ps-')as directory:
        dest=Path(directory)
        for seed in ['aac-ps-matrix-controller-oracles.json','aac-sbr-ps-30-oracles.json']:(dest/seed).write_bytes((SOURCE/seed).read_bytes())
        def movie(*args,**kwargs):
            kwargs['write']=False;result=original_video(*args,**kwargs);(dest/result['file']).write_bytes(result.pop('data'));return result
        try:
            source_generator.DEST=dest;source_generator.video_fixture=movie
            with contextlib.redirect_stdout(io.StringIO()):source_generator.main(frames,include_bits)
            return {p.name:p.read_bytes()for p in dest.iterdir()}
        finally:source_generator.DEST=original_dest;source_generator.video_fixture=original_video

def setup(c,ep):
    aot=c['aot'];rate=c['rate'];ga=field(c['frame_samples']==960,1)+'00'+'10'+ep
    if c['signal']=='explicit':return packed(field(29,5)+frequency(24000)+'0001'+frequency(rate)+field(aot,5)+ga)
    suffix=field(0x2b7,11)+field(5,5)+'1'+frequency(rate)+(field(0x548,11)+'1'if c['signal']=='sync'else '')
    return packed(field(aot,5)+frequency(24000)+'0001'+ga+suffix)

def generate():
    raw=originals();manifest=json.loads(raw['aac-er-ps.json']);source=raw['aac-er-ps-packets.bin'];files={};cases=[];bad=[];padded=[];blob=bytearray();plain=bytearray()
    for c in manifest['cases']:
        core_file=c['core_reference'];files[core_file]=raw[core_file]
        n=c['frame_samples'];rate=c['rate'];out_n=n*rate//24000;original=[];units=[];invalid=[];width=max(r['bytes']for r in c['frames'])+2
        for row in c['frames']:
            start=row['offset'];original.append(source[start:start+row['bytes']]);extra=width-row['bytes']-2;assert 0<=extra<255
            # A length-delimited ancillary element precedes SBR; no padding
            # is smuggled into the unbounded SBR extension's tail.
            ancillary='00100000'+field(extra,8)+'0'*(8*extra);payload=''.join(f'{v:08b}'for v in bytes.fromhex(row['payload']))
            good=packed(row['core_bits']+ancillary+payload);wrong=packed(row['core_bits']+ancillary+'1110'+payload[4:]);assert len(good)==len(wrong)==width
            units.append(good);invalid.append(wrong)
        padded.append(dict(name=c['name'],asc=c['asc'],signal=c['signal'],rate=rate,original=[x.hex()for x in original],padded=[x.hex()for x in units]))
        for frames in range(1,8):
            used=len(units)//frames*frames;count=used//frames
            for family,kind,flags in [('srcpc',0,'mixed'),('srcpc',1,'separate'),('mixed',2,'mixed'),('rs',0,'joined')]:
                lengths=[3,0,5,width*8-8]if family=='srcpc'else[8,0,8,width*8-16];sets,ep=config(lengths,frames,family,kind,flags);asc=setup(c,ep);name=c['name']+f'-f{frames}-{family}-i{kind}-{flags}';files[name+'.asc']=asc
                variants=['clean']+(['crc','sbr-crc']+(['late-sbr-crc']if frames==7 else [])if family=='srcpc'and kind==0 else [])
                for mutation in variants:
                    label=name if mutation=='clean'else name+'-'+mutation;rows=[];packets=[];raw_rows=[];bad_index=1 if mutation=='late-sbr-crc'else 0
                    for i in range(count):
                        children=list(units[i*frames:(i+1)*frames])
                        if mutation in ['sbr-crc','late-sbr-crc']and i==bad_index:children[-1]=invalid[(i+1)*frames-1]
                        packet,selected=protect(children,sets,kind,i,'crc'if mutation=='crc'and i==0 else None);offsets=[]
                        for j,unit in enumerate(children):
                            entry=dict(offset=len(plain),bytes=len(unit),payload=c['frames'][i*frames+j]['payload']);offsets.append(entry);raw_rows.append(entry);plain.extend(unit)
                        rows.append(dict(offset=len(blob),bytes=len(packet),choice=selected,units=offsets));blob.extend(packet);packets.append(packet)
                    case=dict(name=label,aot=c['aot'],n=n,slots=n//64,bands=c['bands'],rate=rate,channels=2,signal=c['signal'],asc=asc.hex(),plain_asc=c['asc'],core_reference=core_file,raw_frames=raw_rows,source_frames=frames,frames=rows,container_rate=rate,container_frame_samples=out_n*frames,frame_samples=out_n,samples=used*out_n,pcm_offset=0)
                    case['video']=label+'.mp4';files[case['video']]=video_fixture([case],blob,2,filename=case['video'],write=False)['data'];case['matroska']=label+'.mka';files[case['matroska']]=mkv(asc,packets,rate,out_n*frames,2)
                    if mutation=='clean':cases.append(case)
                    else:case.update(good=name,bad_index=bad_index,error='EP class CRC mismatch'if mutation=='crc'else'ER AAC SBR CRC is forbidden');bad.append(case)
    files['packets.bin']=bytes(blob);files['plain-packets.bin']=bytes(plain);files['manifest.json']=(json.dumps(dict(cases=cases,malformed=bad,padded=padded,provenance='Own fourteen-frame ER-LC/LTP sparse-core/LTP scalar oracle, authored PS payloads, valid pre-SBR ancillary fill and independent EP polynomial/matrix protection. No private media, foreign codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
