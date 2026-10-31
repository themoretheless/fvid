#!/usr/bin/env python3
"""Own high-rate128-band GES video regression corpus; explicit offline generation."""
import generate_aac_eld_mps128_fixtures as base
if __name__=='__main__':
 base.OUT=base.FIX/'aac-eld-mps128-ges'
 base.SHAPE=2
 base.main()
