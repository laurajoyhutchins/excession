# Release procedure

Excession uses one native CLI and publishes immutable archives through the
existing [Portable CLI workflow](../.github/workflows/portable-cli.yml).
**No published release is implied by the crate version or this procedure.**

## Gate

1. Select an exact clean main commit and confirm its full `Verify` run.
2. Run the full portability matrix (label a review PR `ci:full-portability`
   **before pushing its next commit**, or invoke the existing manual Portable
   CLI workflow). The default PR trigger does not run on label changes alone;
   the next PR synchronize event must see the label. Verify Linux x86-64,
   Linux ARM64, macOS Apple Silicon, macOS Intel, Windows, and musl/Alpine
   independently. Do not include a target that failed.
3. Check the dedicated independent upstream source oracle, including the
   public `analyze → impact → explain → audit` path on pinned h11 and Axios.
4. Review the crate version, source commit, portability support, third-party
   notices, and release notes. A draft schema is not an implementation guarantee.

## Tag and publish

After the gate, create the matching version tag using a trusted, authorized
GitHub client, for example for crate version `0.2.0`:

```sh
git checkout main
git pull --ff-only
git tag -a v0.2.0 -m "Excession v0.2.0"
git push origin v0.2.0
```

The tag must identify the exact reviewed commit. Tag builds run the full
platform matrix and musl check; only after all required jobs pass does
`publish` upload archives, adjacent SHA-256 checksums, and
`release-manifest.json`. The manifest binds each artifact to its source
commit and TypeScript sidecar digest and currently declares
`attestation_status=NOT_ATTESTED`. Do not describe checksums as
publisher signatures or SLSA attestations.

Published archives are never overwritten. A hotfix requires a new source
commit and a new release version/tag. If a tag job fails, investigate and
publish a corrected version rather than replacing a public artifact.

## Install, verify, and remove

Retrieve the archive matching the platform from
[GitHub Releases](https://github.com/laurajoyhutchins/excession/releases).
If no release is listed, build from pinned source instead.

On Linux/macOS, verify the adjacent checksum from a trusted release page:

```sh
sha256sum -c excession-v0.2.0-PLATFORM.tar.gz.sha256
mkdir -p "$HOME/.local/opt"
tar -xzf excession-v0.2.0-PLATFORM.tar.gz -C "$HOME/.local/opt"
"$HOME/.local/opt/excession/excession" doctor
```

Replace `PLATFORM` with the actual archive target. On macOS use
`shasum -a 256` for checksum verification if GNU `sha256sum`
is unavailable. On Windows, use PowerShell `Get-FileHash -Algorithm SHA256`
and compare to the published checksum; extract the ZIP without moving the
executable away from its bundled `lib/typescript.js` sidecar.

Upgrade by extracting a newly verified versioned archive to a new directory
and switching the path atomically, keeping the previous installation for
rollback. Uninstall by removing the chosen extracted installation directory;
the CLI does not require a system service. Its user cache (reported by
`discover`) is independent and can be removed separately.

## Supported scope

Static Python and JavaScript/TypeScript dependencies are *candidate*
relationships. Configured C/C++, Rust, Go, SCIP symbol and scanner workflows
require explicit native inputs and remain separately scoped. Source replay
authenticates pinned observations; it does not prove an impact safe, a
refactoring correct, or a scanner candidate exploitable. Git SHA-256 object
repositories are not yet supported.
