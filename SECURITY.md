# Security policy (initial)

Excession consumes Git source and may invoke locally installed analyzers, parsers, compilers and scanners. **It is not an execution sandbox.** Do not run trusted-local scanner/compiler modes on untrusted repositories without independent isolation.

## Reporting a vulnerability

Check this repository's **Security → Advisories** page for GitHub's private vulnerability-reporting option. If available, report through that private channel. If unavailable, open an issue **without technical exploit details** requesting a private reporting channel from the maintainer. Do not publish exploit payloads, credentials, or private source in a public issue.

No release support duration or response-time guarantee has been established yet.

## Evidence semantics

CodeQL/Semgrep source ingestion and report replay establish source provenance and recorded output, **not** that a finding is exploitable. Reported security candidates remain unverified until independent behavioral validation is performed.

Git SHA-1 repository OIDs identify repository objects under current supported Git format. A self-consistent content digest is not a signature or a guarantee that a producer executed honestly.
