#!/usr/bin/env python3
"""A/B full manifest output against the previous authenticated implementation.

Reuses the native differential harness for fixed Git/SQLite corpora, exact
canonical bytes, matched view IDs and interleaved full/compact query timings.
"""
import canonical_ab

canonical_ab.BASELINE_SHA = "670c3e4b80d8cc887719c4a020cd254489a2b83d"

if __name__ == "__main__":
    canonical_ab.main()
