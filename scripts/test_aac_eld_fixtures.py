"""Regenerate own ELD fixtures in memory, with external codecs/network forbidden.

Run separately from ordinary Rust tests: python3 scripts/test_aac_eld_fixtures.py.
"""
import json
import socket
import subprocess

import generate_aac_eld_fixtures as eld


def forbidden(*args, **kwargs):
    raise AssertionError("ELD fixture generation must not use processes or network")


def main():
    subprocess.Popen = forbidden
    socket.socket = forbidden
    files = eld.generate()
    actual = {p.name for p in eld.OUT.iterdir() if p.is_file()}
    assert set(files) == actual, (set(files) - actual, actual - set(files))
    for name, data in files.items():
        assert (eld.OUT / name).read_bytes() == data, name
    manifest = json.loads(files['manifest.json'])
    cases = manifest['cases']
    assert len({c['name'] for c in cases}) == len(cases) == 60
    for case in cases:
        assert files[case['video']]
        assert files[case['matroska']]
        assert len(files[case['name'] + '.f32le']) == case['samples'] * case['channels'] * 4
    print(f"PASS: {len(cases)} cases; {len(files)} deterministic offline files; "
          f"{sum(map(len, files.values()))} bytes")


if __name__ == '__main__':
    main()
