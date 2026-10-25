"""Short owned Dolby MP4 witnesses with normative 16-bit little-endian words."""
from pathlib import Path
import struct,json
from generate_mp4_fragment_description_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/dolby-word-order'
ROOT=OUT.parent
def audio_ranges(data):
    ranges=[]
    def scan(body,audio=False):
        for k,b in atoms(body):
            if k==b'trak':scan(b,b'soun'in b)
            elif k in [b'moov',b'mdia',b'minf']:scan(b,audio)
            elif k==b'stbl' and audio:
                parts=dict(atoms(b));off=parts[b'stco'];assert int.from_bytes(off[4:8],'big')==1
                at=int.from_bytes(off[8:12],'big');size=parts[b'stsz'];fixed,count=struct.unpack_from('>II',size,4)
                sizes=[fixed]*count if fixed else list(struct.unpack_from('>'+str(count)+'I',size,12))
                for i,n in enumerate(sizes):
                    ranges.append((at,n))
                    at+=n
    scan(data);return ranges
def swap_audio(data,mixed=False):
    ranges=audio_ranges(data);out=bytearray(data)
    ordinal=0
    for at,n in ranges:
        end=at+n
        while at<end:
            assert data[at:at+2]==b'\x0b\x77'
            if data[at+5]>>3<=8:
                assert data[at+4]>>6==0
                bitrate=[32,40,48,56,64,80,96,112,128,160,192,224,256,320,384,448,512,576,640][(data[at+4]&63)//2]
                size=bitrate*4
            else:size=((data[at+2]&7)*256+data[at+3]+1)*2
            assert size%2==0 and at+size<=end
            if not mixed or ordinal%2==1:
                for i in range(at,at+size,2):out[i],out[i+1]=out[i+1],out[i]
            ordinal+=1;at+=size
    return bytes(out)
def generate():
    sources=[('eac3-corrupt-unselected','eac3-mp4-programmes/corrupt-unselected-p1.mp4',1),('ac3-bad-first','mp4-ac3-sample-clock/bad-first-exact.mp4',0),('ac3-bad-first-coarse','mp4-ac3-sample-clock/one-packet-bad-first-900-floor.mp4',0),('ac3','mp4-ac3-sample-clock/exact.mp4',0),('ac3-coarse','mp4-ac3-sample-clock/ac3-900-floor.mp4',0),('ac3-grouped','mp4-ac3-sample-clock/one-packet-exact.mp4',0),('eac3','eac3-mp4-programmes/dual-48000.mp4',7),('eac3-expanded','eac3-mp4-programmes/expanded-48000-b3.mp4',7),('eac3-coarse','mp4-eac3-programme-clock/expanded-44100-b6-900-floor.mp4',7)]
    from generate_eac3_bsid import movie
    files={};cases=[]
    for name,path,p in sources:
        b=(ROOT/path).read_bytes();files[name+'-be.mp4']=b
        controls=[b[at:at+n]for at,n in audio_ranges(b)]
        rate=44100 if name=='eac3-coarse' else 48000
        blocks=18 if name in ['ac3-grouped','ac3-bad-first-coarse'] else 6
        channels=6 if name.startswith('ac3') else 8 if name in ['eac3-expanded','eac3-coarse'] else 3
        files[name+'-be.raw']=b''.join(controls)
        files[name+'-be.mkv']=movie(controls,blocks=blocks,channels=channels,rate=rate,codec='A_AC3' if name.startswith('ac3') else 'A_EAC3')
        for mixed in [False,True]:
            key=name+('-mixed'if mixed else '-le')+'.mp4';files[key]=swap_audio(b,mixed)
            cases.append(dict(file=key,control=name+'-be.mp4',programme=p))
            packets=[files[key][at:at+n]for at,n in audio_ranges(files[key])]
            for suffix,encoded in [('.raw',b''.join(packets)),('.mkv',movie(packets,blocks=blocks,channels=channels,rate=rate,codec='A_AC3' if name.startswith('ac3') else 'A_EAC3'))]:
                extra=name+('-mixed'if mixed else '-le')+suffix;files[extra]=encoded
                cases.append(dict(file=extra,control=name+'-be'+suffix,programme=p))
    # Compact frames expose raw-byte frmsiz probes: e.g. 912 bytes becomes
    # 3588 when the LE header is read before word normalization.
    from generate_eac3_dependent import packet
    from generate_eac3_player_stereo import audiovisual
    from generate_eac3_playback import patch_crc
    for silent,mode in [(True,1),(False,2)]:
        for programme in [0,7]:
            name='eac3-short-'+('silent'if silent else'tone')+'-p'+str(programme)
            selected,_=packet(mode,False,programme,0,'mixed',None,False,blocks_override=3,allocated=not silent,compact=True,divisor=16)
            selected=bytearray(selected);selected[2]=(selected[2]&0xc7)|(programme<<3);selected=patch_crc(selected)
            other,_=packet(1,False,0,0,'mixed',None,False,blocks_override=3,allocated=False,compact=True)
            packets=[(other if programme else b'')+selected]*3
            mp4=audiovisual(packets,mode,False,1 if silent else 2)
            controls={'.mp4':mp4,'.raw':b''.join(packets),'.mkv':movie(packets,blocks=3,channels=1 if silent else 2)}
            for suffix,b in controls.items():files[name+'-be'+suffix]=b
            for mixed in [False,True]:
                variant='-mixed'if mixed else'-le';encoded=swap_audio(mp4,mixed)
                changed=[encoded[at:at+n]for at,n in audio_ranges(encoded)]
                variants={'.mp4':encoded,'.raw':b''.join(changed),'.mkv':movie(changed,blocks=3,channels=1 if silent else 2)}
                for suffix,b in variants.items():
                    key=name+variant+suffix;files[key]=b
                    cases.append(dict(file=key,control=name+'-be'+suffix,programme=programme))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned neutral VP9, repository-authored AC-3 tone and authored E-AC-3 spectra. Swap 16-bit audio words only; MP4/video/PTS/CRC logical values unchanged. No private media, FFmpeg, subprocess or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
