# Origin and migration

Excession inherits the RelGit Security v0.1.0 research implementation, which itself builds upon the RelGit Rust v0.19 research code. The original six local Git commits, with their source trees and checks, remain in the original `relgit-security-0.1.0.bundle`.

The GitHub destination was independently initialized with a MIT-license commit. Excession imports the six original changes in order on top of that existing commit; Git SHA identities of the imported commits differ because the ancestry is different. The source blobs remain Git-content-addressable, and each imported commit is separately inspectable.

The Rust executable is still named `relgit` for compatibility. Scanner integrations use pre-existing CodeQL and Semgrep formats; native Go and Python tests are bounded fixtures, not proof of exploitable vulnerabilities. The public code includes test bundles and source snapshots from upstream libraries with provenance documented in `THIRD_PARTY.md`.

`v0.1.0` labels the initial local RelGit Security release. The Excession presentation-name change is intentionally a new commit rather than a history rewrite.
