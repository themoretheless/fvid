#!/usr/bin/env python3
"""Explicit independent ISO GES128 component oracle,never invoked by tests."""
import sys,hashlib
from pathlib import Path
import generate_aac_ld_mps_ges_reference as base
if __name__=='__main__':
 assert hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest()=='b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83'
 base.main(sys.argv[1],base.ROOT/'tests/fixtures/playback-errors/aac-eld-mps128-ges','aac-eld-mps128-ges-reference-2026-10-31.json')
