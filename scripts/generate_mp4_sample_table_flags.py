"""Author zero/reserved sample-table flag companions; no encoder or random mutation."""
from pathlib import Path
import json, struct
from generate_mp4_audio_clock_regressions import generate as clocks
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom, word
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-sample-table-flags'
TAGS = [b'stsd', b'stts', b'ctts', b'stsc', b'stsz', b'stz2', b'stco', b'co64', b'stss']
def generate():
    sources = clocks()
    files = {'expected.f32le': sources['expected.f32le']}
    cases, bad = [], []
    for family in ['video', 'pcm', 'alac']:
        source = sources['pcm-1000.mp4' if family != 'alac' else 'alac-1000.mp4']
        for tag in TAGS:
            def build(flags):
                def walk(data, selected=False):
                    out = b''
                    for kind, body in boxes(data):
                        active = selected
                        if kind == b'trak':
                            mdia = dict(boxes(body))[b'mdia']
                            handler = dict(boxes(mdia))[b'hdlr'][8:12]
                            active = handler == (b'vide' if family == 'video' else b'soun')
                        if kind in [b'moov', b'trak', b'mdia', b'minf']:
                            body = walk(body, active)
                        elif kind == b'stbl' and active:
                            children = list(boxes(body))
                            if tag == b'ctts':
                                children = [(k,b) for k,b in children if k != tag] + [(tag,word(0x01000000,1,3,0))]
                            elif tag == b'stss' and not any(k == tag for k,b in children):
                                children += [(tag,word(0,3,1,2,3))]
                            elif tag == b'co64':
                                children = [(tag, word(0,1)+struct.pack('>Q',int.from_bytes(b[8:12],'big'))) if k == b'stco' else (k,b) for k,b in children]
                            elif tag == b'stz2':
                                changed = []
                                for k,b in children:
                                    if k != b'stsz': changed.append((k,b)); continue
                                    fixed,count = struct.unpack('>II',b[4:12])
                                    sizes = [fixed]*count if fixed else list(struct.unpack('>'+str(count)+'I',b[12:]))
                                    assert all(0 < n <= 65535 for n in sizes)
                                    changed.append((tag,word(0,16,count)+struct.pack('>'+str(count)+'H',*sizes)))
                                children = changed
                            found = 0
                            output = []
                            for k,b in children:
                                if k == tag:
                                    found += 1
                                    b = b[:1]+flags.to_bytes(3,'big')+b[4:]
                                output.append(atom(k,b))
                            assert found == 1
                            body = b''.join(output)
                        out += atom(kind,body)
                    return out
                return walk(source)
            control = f'{family}-{tag.decode()}-zero.mp4'
            files[control] = build(0)
            cases.append(dict(file=control,family=family,tag=tag.decode()))
            for flag in [1,0x800000,0xffffff]:
                name = f'{family}-{tag.decode()}-{flag:06x}.mp4'
                files[name] = build(flag)
                bad.append(dict(file=name,control=control,family=family,tag=tag.decode(),flags=flag))
    files['manifest.json']=(json.dumps(dict(provenance='Authored60ms16x16 VP9 plus8kHz mono integer PCM/raw ALAC. Full sample-table records and media are retained in zero-flag controls; only the explicitly authored reserved flags differ. No private media, encoder, random audit or network.',cases=cases,bad=bad),indent=2)+'\n').encode()
    return files
if __name__ == '__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items(): (OUT/name).write_bytes(data)
