# Independent product acceptance

A Rust integration test creates its own pinned Git repository with two author-reviewed imports, an unrelated module, and a deliberately changed target. It checks the exact expected relationship set without using Excession's SQL graph as its oracle, authenticates both sides against Git's own object IDs, and verifies the precise candidate-affected source set.

This is a **bounded independent synthetic fixture**, not proof of cross-language completeness or an unfamiliar production-repository study. The separate real-repository and installed-archive acceptance requirements remain tracked by #84 and #113.

The versioned acceptance-case schema documents future held-out pinned source cases and adversarial mutations. Auto-ontology evaluation must reuse #95's independent concept baseline, not be treated as native dependency validation.
