#!/usr/bin/env python3
"""Measure direct projection IDs against the immediately preceding implementation.

The underlying canonical A/B harness supplies identical Git/index datasets,
byte-for-byte manifest parity, interleaved process timings and internal
canonical-ID timings. Only the pinned baseline commit changes.
"""
import canonical_ab

canonical_ab.BASELINE_SHA = "90cb4d5c1cbe7fde704a6d9189aa03cb5eff7a66"

if __name__ == "__main__":
    canonical_ab.main()
