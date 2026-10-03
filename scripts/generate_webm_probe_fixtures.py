#!/usr/bin/env python3
"""Synthetic Matroska probe controls; no codec programs or private media."""
from generate_pcm_precision_fixtures import ROOT, element

if __name__ == '__main__':
    ROOT.mkdir(parents=True,exist_ok=True)
    header = element('1a45dfa3',element('4282',b'matroska'))
    base = element('d7',b'\x01')+element('83',b'\x01')+element('86',b'V_FFV1')
    base += element('e0',element('b0',b'\x10')+element('ba',b'\x08'))
    cluster = element('1f43b675',element('e7',b'\0')+element('a3',b'\x81\0\0\x80\x01\x02\x03'))
    clear = header+element('18538067',element('1654ae6b',element('ae',base))+cluster)
    (ROOT/'webm-probe-clear.mkv').write_bytes(clear)
    # Header-removal compression is a known container extension, not corruption.
    encoding = element('6d80',element('6240',element('5034',element('4254',b'\x03')+element('4255',b'\x01'))))
    encoded = header+element('18538067',element('1654ae6b',element('ae',base+encoding))+cluster)
    (ROOT/'webm-probe-content-encoding.mkv').write_bytes(encoded)
    # A complete synthetic header followed by a Segment whose extent exceeds EOF.
    (ROOT/'webm-probe-segment-beyond-file.mkv').write_bytes(clear[:len(header)+4]+b'\x10\0\x01\0'+clear[len(header)+8:])

    lace = element('1f43b675',element('e7',b'\0')+element('a3',b'\x81\0\0\x82\x01\x01\x01\x02'))
    (ROOT/'webm-probe-laced.mkv').write_bytes(header+element('18538067',element('1654ae6b',element('ae',base))+lace))
