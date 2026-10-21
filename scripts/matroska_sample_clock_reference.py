"""Fixture-only frozen audio vectors; extraction for explicit reference benchmarks.
No encoder, subprocess, network or production decoder is used here.
"""
import hashlib
import json
import struct
from pathlib import Path
from avc_fixture_mp4 import elements, vint

ROOT = Path(__file__).resolve().parents[1]
REFERENCES = ROOT / 'tests/fixtures/audio/matroska-sample-clock-references'
ENCODERS = [('flac', 'flac'), ('alac', 'alac'), ('mp3', 'libmp3lame'),
            ('mp2', 'mp2'), ('vorbis', 'vorbis'), ('ac3', 'ac3')]

def sha(data): return hashlib.sha256(data).hexdigest()
def integer(data): return int.from_bytes(data, 'big')
def floating(data): return struct.unpack('>f' if len(data) == 4 else '>d', data)[0]

def extract(data, name, encoder):
    """Extract fixed synthetic reference audio without changing coded bits."""
    segment = list(elements(dict(elements(data))[0x18538067]))
    info = dict(elements(next(v for k, v in segment if k == 0x1549A966)))
    entries = [dict(elements(v)) for k, v in elements(next(v for k, v in segment if k == 0x1654AE6B)) if k == 0xAE]
    tracks = [t for t in entries if integer(t[0x83]) == 2]
    assert len(tracks) == 1
    track = tracks[0]; audio = dict(elements(track[0xE1]))
    assert 0x6D80 not in track, 'reference extraction does not strip content encodings'
    assert 0x23314F not in track or floating(track[0x23314F]) == 1.0
    assert 0x78B5 not in audio or floating(audio[0x78B5]) == floating(audio[0xB5])
    number = integer(track[0xD7]); private = track.get(0x63A2, b'')
    packets, payload = [], bytearray()
    for kind, cluster in segment:
        if kind != 0x1F43B675: continue
        fields = list(elements(cluster)); timestamp = integer(next(v for k, v in fields if k == 0xE7))
        for tag, body in fields:
            group = list(elements(body)) if tag == 0xA0 else []
            block = next((v for k, v in group if k == 0xA1), None) if group else body if tag == 0xA3 else None
            if block is None: continue
            identity, at = vint(block, 0)
            if identity != number: continue
            assert len(block) >= at + 3 and block[at+2] & 6 == 0, 'reference needs un-laced complete audio frames'
            assert not any(k in [0xA4, 0x75A1] for k, _ in group), 'unsupported reference CodecState/additions'
            relative = struct.unpack_from('>h', block, at)[0]
            raw = block[at+3:]; values = dict(group)
            refs = [int.from_bytes(v, 'big', signed=True) for k, v in group if k == 0xFB]
            packets.append(dict(offset=len(payload), size=len(raw), timestamp_ticks=timestamp+relative,
                                group=bool(group), flags=block[at+2], references_ticks=refs,
                                duration_ticks=integer(values[0x9B]) if 0x9B in values else None,
                                discard_padding_ns=int.from_bytes(values.get(0x75A2, b'\0'), 'big', signed=True)))
            payload.extend(raw)
    assert packets and all(p['size'] for p in packets)
    assert floating(audio[0xB5]) == 48000.0
    assert integer(audio.get(0x9F, b'\1')) == (2 if name == 'vorbis' else 1)
    metadata = dict(name=name, reference_encoder=encoder, source_movie_sha256=sha(data),
                    codec=track[0x86].decode(), sample_rate=48000, channels=integer(audio.get(0x9F, b'\1')),
                    bit_depth=integer(audio[0x6264]) if 0x6264 in audio else None,
                    codec_delay_ns=integer(track.get(0x56AA, b'\0')),
                    seek_preroll_ns=integer(track.get(0x56BB, b'\0')),
                    default_duration_ns=integer(track.get(0x23E383, b'\0')),
                    timestamp_scale_ns=integer(info.get(0x2AD7B1, b'\x0f\x42\x40')),
                    duration_ticks=floating(info[0x4489]), packets=packets,
                    payload_file=f'{name}.packets', payload_sha256=sha(payload),
                    private_file=f'{name}.private', private_sha256=sha(private))
    return metadata, {metadata['payload_file']: bytes(payload), metadata['private_file']: private}

def capture(movies):
    files, cases = {}, []
    for name, encoder in ENCODERS:
        case, source = extract(movies[name], name, encoder)
        cases.append(case); files.update(source)
    files['manifest.json'] = (json.dumps(dict(
        provenance='Frozen encoded audio only from the original authored 240 ms / 48 kHz 440 Hz tone references. '
                   'Codec-private bytes are synthetic configuration, not private source media. No container/video bytes are stored. '
                   'External reference refresh: benches/generate_matroska_audio_sample_clock_reference.py.',
        cases=cases), indent=2) + '\n').encode()
    return files

def load(base=REFERENCES):
    manifest = json.loads((base / 'manifest.json').read_bytes())
    assert [c['name'] for c in manifest['cases']] == [name for name, _ in ENCODERS]
    result = []
    for case in manifest['cases']:
        payload = (base / case['payload_file']).read_bytes(); private = (base / case['private_file']).read_bytes()
        assert sha(payload) == case['payload_sha256'] and sha(private) == case['private_sha256']
        packets = []; end = 0
        for packet in case['packets']:
            assert packet['offset'] == end
            end += packet['size']; assert end <= len(payload)
            packets.append(payload[packet['offset']:end])
        assert end == len(payload)
        result.append((case, private, packets))
    return result
