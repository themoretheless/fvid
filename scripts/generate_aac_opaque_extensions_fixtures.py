#!/usr/bin/env python3
"""Own forward-compatible opaque AAC FIL/ER extensions; no foreign codec."""
from generate_aac_er_extensions_fixtures import main
if __name__ == '__main__':
    main(opaque=True)
