#!/usr/bin/env python3
"""Own synthetic MOV descriptions for probe selection; no external codecs."""
import struct
from generate_pcm_precision_fixtures import ROOT, quicktime_pcm

if __name__ == '__main__':
    ROOT.mkdir(parents=True, exist_ok=True)
    valid = quicktime_pcm(b'sowt',16,bytes(8))
    unknown = quicktime_pcm(b'zzzz',16,bytes(8))
    (ROOT/'probe-unknown-entry.mov').write_bytes(unknown)
    broken = bytearray(valid)
    struct.pack_into('>I',broken,0,len(broken)+1)
    (ROOT/'probe-mdat-beyond-file.mov').write_bytes(broken)
    # A represented PCM track plus an unknown entry must not become a reindexed
    # one-stream inventory. Both tracks refer only to the synthetic silent mdat.
    at = unknown.index(b'trak')-4
    size = struct.unpack_from('>I',unknown,at)[0]
    track = bytearray(unknown[at:at+size])
    struct.pack_into('>I',track,track.index(b'tkhd')+16,2)
    mixed = bytearray(valid)
    moov = mixed.index(b'moov')-4
    old_size = struct.unpack_from('>I',mixed,moov)[0]
    struct.pack_into('>I',mixed,moov,old_size+len(track))
    mixed += track
    (ROOT/'probe-mixed-unknown-entry.mov').write_bytes(mixed)
