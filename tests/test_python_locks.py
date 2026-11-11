"""The Linux CI wheel lock must cover every declared non-Windows dependency."""
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
PIN = re.compile(r"^([A-Za-z0-9_.-]+)==([A-Za-z0-9_.-]+)")
HASH = re.compile(r"^[ ]+--hash=sha256:([0-9a-f]{64})$")
def packages(path, locked=False):
    lines = (ROOT / path).read_text().splitlines()
    output = {}
    last = None
    for raw in lines:
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        if raw.startswith(" ") and locked:
            digest = HASH.fullmatch(raw)
            if digest is None or last is None:
                raise AssertionError("invalid SHA-256 wheel lock line: " + raw)
            output[last][1].append(digest.group(1))
            last = None
            continue
        text = raw.rstrip().removesuffix("\\").strip()
        match = PIN.fullmatch(text) if locked else PIN.match(text)
        if match is None:
            raise AssertionError("unexpected unpinned requirement: " + raw)
        name, version = match.groups()
        if not locked and 'sys_platform == "win32"' in raw:
            continue
        name = name.lower().replace("_", "-").replace(".", "-")
        if name in output:
            raise AssertionError("duplicate package: " + name)
        output[name] = [version, []]
        last = name
    if locked and any(len(v[1]) != 1 for v in output.values()):
        raise AssertionError("missing wheel hash")
    return output


class LockCompletenessTests(unittest.TestCase):
    def test_research_lock_matches_declared_requirements(self):
        source = packages("requirements/research.txt")
        locked = packages("requirements/locks/research-linux-cp313-x86_64.txt", True)
        self.assertEqual({name: v[0] for name, v in source.items()},
                         {name: v[0] for name, v in locked.items()})

    def test_syntax_lock_matches_declared_requirements(self):
        source = packages("requirements/syntax.txt")
        locked = packages("requirements/locks/syntax-linux-cp313-x86_64.txt", True)
        self.assertEqual({name: v[0] for name, v in source.items()},
                         {name: v[0] for name, v in locked.items()})


if __name__ == "__main__":
    unittest.main()
