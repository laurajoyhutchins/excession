"""Narrow language-native resolver behind Excession's Rust input authority.

Input: <staged-root> <entry> <language> <trusted TypeScript helper>.
Output: one JSON object with source/configuration paths and resolver assumptions.

This module does not read Git history, construct evidence contracts, validate
hashes, or stage files. Those responsibilities belong to the Rust core.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path, PurePosixPath


class Unsupported(ValueError):
    pass


def run(argv, cwd=None, env=None):
    try:
        p = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, timeout=45)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise Unsupported(f"native resolver unavailable or timed out: {exc}") from exc
    if len(p.stdout) > 16 * 1024 * 1024 or len(p.stderr) > 16 * 1024 * 1024:
        raise Unsupported("native resolver exceeded 16 MiB output limit")
    if p.returncode:
        detail = (p.stderr or p.stdout)[:700].decode(errors="replace").strip()
        raise Unsupported(f"native resolver exited {p.returncode}: {detail or 'no diagnostics'}")
    return p.stdout


def find(name):
    executable = shutil.which(name)
    if executable is None:
        raise Unsupported(f"native adapter requires {name}")
    # Keep invocation path and argv[0] intact: rustc is often a rustup shim.
    return Path(executable).absolute()


def relative(root, filename):
    try:
        rel = Path(filename).resolve(strict=True).relative_to(root.resolve()).as_posix()
    except (ValueError, OSError) as exc:
        raise Unsupported(f"dependency outside pinned Git snapshot: {filename}") from exc
    if rel == "." or any(x in ("", ".", "..") for x in rel.split("/")):
        raise Unsupported("invalid resolved dependency path")
    return rel


def go(root, entry):
    if not (root / "go.mod").is_file() or (root / "go.work").exists():
        raise Unsupported("Go requires tracked go.mod without go.work")
    dirname = PurePosixPath(entry).parent.as_posix()
    env = {"PATH": os.environ.get("PATH", "/usr/local/go/bin:/usr/bin:/bin"),
           "HOME": str(root.parent), "GOCACHE": str(root.parent / "go-cache"),
           "GOMODCACHE": str(root.parent / "go-mod-cache"),
           "GOPROXY": "off", "GOSUMDB": "off", "GOTOOLCHAIN": "local",
           "GOWORK": "off", "CGO_ENABLED": "0", "GOFLAGS": "-mod=readonly",
           "LC_ALL": "C", "TZ": "UTC"}
    output = run([str(find("go")), "list", "-deps", "-test", "-json",
                  "./" if dirname == "." else "./" + dirname], cwd=root, env=env)
    decoder = json.JSONDecoder()
    text = output.decode()
    offset = 0
    selected, package_dirs = set(), set()
    while offset < len(text):
        while offset < len(text) and text[offset].isspace():
            offset += 1
        if offset == len(text):
            break
        item, offset = decoder.raw_decode(text, offset)
        if item.get("Error") or item.get("DepsErrors"):
            raise Unsupported("Go native resolver reported dependency errors")
        if item.get("Standard") or str(item.get("ImportPath", "")).endswith(".test"):
            continue
        directory = item.get("Dir")
        if not directory:
            raise Unsupported("native Go package has no directory")
        package_dirs.add("." if Path(directory) == root else relative(root, directory))
        if any(item.get(key) for key in ("CgoFiles", "SwigFiles", "SwigCXXFiles")):
            raise Unsupported("Go cgo/SWIG needs separate toolchain effects")
        for key in ("GoFiles", "TestGoFiles", "XTestGoFiles", "SFiles",
                    "CFiles", "CXXFiles", "HFiles", "EmbedFiles",
                    "TestEmbedFiles", "XTestEmbedFiles", "SysoFiles"):
            for filename in item.get(key, []):
                selected.add(relative(root, Path(directory) / filename))
    # Include build-tag alternatives in any reached local package.
    for filename in root.rglob("*.go"):
        if filename.is_file() and filename.relative_to(root).parent.as_posix() in package_dirs:
            selected.add(filename.relative_to(root).as_posix())
    if entry not in selected:
        raise Unsupported("entry not reached by go list")
    return selected, {p for p in ("go.mod", "go.sum") if (root / p).is_file()}, {
        "go-flags": "CGO_ENABLED=0,GOWORK=off,GOPROXY=off"}


def typescript(root, entry, helper):
    # Resolve only for locating TypeScript's adjacent library, not execution.
    tsc = find("tsc").resolve()
    ts_api = tsc.parent.parent / "lib" / "typescript.js"
    if not ts_api.is_file():
        raise Unsupported("TypeScript compiler API missing beside tsc")
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(root.parent),
           "NODE_OPTIONS": "", "NODE_PATH": "", "LC_ALL": "C", "TZ": "UTC"}
    result = json.loads(run([str(find("node")), str(helper), str(ts_api),
                             str(root), entry], cwd=root, env=env))
    import hashlib
    sha = hashlib.sha256(ts_api.read_bytes()).hexdigest()
    return set(result["sources"]), set(result["configuration"]), {
        "typescript-api-sha256": sha,
        "typescript-options": result["options_sha256"]}


def rust(root, entry):
    config = root / "Cargo.toml"
    if not config.is_file():
        raise Unsupported("Rust requires tracked Cargo.toml")
    manifest = tomllib.loads(config.read_text())
    if "workspace" in manifest or any(manifest.get(key) for key in (
            "dependencies", "dev-dependencies", "build-dependencies", "patch", "replace")):
        raise Unsupported("Rust workspace/external dependencies need Cargo-aware adapter")
    package = manifest.get("package", {})
    if not isinstance(package, dict) or package.get("build") is not None or (root / "build.rs").exists():
        raise Unsupported("Rust build script is unsupported")
    if (root / ".cargo/config.toml").exists():
        raise Unsupported("Rust Cargo configuration needs additional interpretation")
    edition = package.get("edition", "2015")
    if edition not in ("2015", "2018", "2021", "2024") or entry not in ("src/lib.rs", "src/main.rs"):
        raise Unsupported("unsupported Rust crate root/edition")
    for filename in root.rglob("*.rs"):
        if re.search(r"\b(?:env|option_env|concat|include)!\s*\(", filename.read_text(errors="replace")):
            raise Unsupported("Rust compile-time environment or dynamic include unsupported")
    depfile = root.parent / "rust-dependencies.d"
    selected = os.environ.get("EXCESSION_NATIVE_RUSTC")
    if not selected:
        raise Unsupported("Rust authority did not pin a compiler executable")
    rustc = Path(selected)
    if not rustc.is_absolute() or not rustc.is_file():
        raise Unsupported("Rust authority supplied an invalid compiler path")
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(root.parent),
           "LC_ALL": "C", "TZ": "UTC", "CARGO_MANIFEST_DIR": str(root),
           "CARGO_PKG_NAME": str(package.get("name", "probe"))}
    run([str(rustc), "--crate-name", "excession_native_probe", "--crate-type",
         "lib" if entry == "src/lib.rs" else "bin", "--edition", edition,
         "--emit", f"dep-info={depfile}", str(root / entry)], cwd=root, env=env)
    first = depfile.read_text().splitlines()[0]
    if ": " not in first or any(x in first for x in ("\\ ", "\\#", "\\:")):
        raise Unsupported("unsupported rustc dep-info encoding")
    selected = {relative(root, p) for p in first.split(": ", 1)[1].split()}
    if entry not in selected:
        raise Unsupported("rustc dep-info omitted entry")
    return selected, {p for p in ("Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml")
                      if (root / p).is_file()}, {
        "rustc-edition": edition, "rustc-mode": "dep-info-no-externs"}


def main():
    if len(sys.argv) != 5:
        raise Unsupported("expected staged root, entry, language, and helper")
    root, entry, lang, helper = Path(sys.argv[1]).resolve(), sys.argv[2], sys.argv[3], Path(sys.argv[4])
    if lang == "go":
        src, config, assumptions = go(root, entry)
    elif lang in ("javascript", "typescript"):
        src, config, assumptions = typescript(root, entry, helper)
    elif lang == "rust":
        src, config, assumptions = rust(root, entry)
    else:
        raise Unsupported("unsupported language")
    print(json.dumps({"sources": sorted(src), "configuration": sorted(config),
                      "assumptions": assumptions}, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    try:
        main()
    except (Unsupported, ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print("UNSUPPORTED: " + str(error), file=sys.stderr)
        sys.exit(2)
