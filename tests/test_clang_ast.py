"""Regression tests for Clang-backed translation-unit observations."""
import importlib.util
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

MODULE = Path(__file__).resolve().parents[1] / "core/adapters/clang_ast.py"
spec = importlib.util.spec_from_file_location("clang_ast", MODULE)
clang = importlib.util.module_from_spec(spec)
spec.loader.exec_module(clang)


class ClangDatabaseTests(unittest.TestCase):
    def test_parse_compilation_flags_without_shell(self):
        cwd = Path("/tmp")
        src = cwd / "sample.c"
        args = ["clang", "-DTEST=1", "-I", "include", "-c", "sample.c",
                "-o", "sample.o", "-MMD", "-MF", "sample.d",
                "-Wp,-MMD,build/.sample.o.d"]
        flags = clang.analysis_flags(args, src, cwd)
        self.assertIn("-DTEST=1", flags)
        self.assertIn("include", flags)
        self.assertIn(str(src), flags)
        self.assertNotIn("-o", flags)
        self.assertNotIn("sample.o", flags)
        self.assertNotIn("sample.d", flags)
        self.assertNotIn("-MMD", flags)
        self.assertNotIn("-Wp,-MMD,build/.sample.o.d", flags)

    def test_reject_compiler_extensions_and_response_files(self):
        src, cwd = Path("/tmp/a.c"), Path("/tmp")
        for argv in [
            ["clang", "-Xclang", "-load", "plugin.so", "a.c"],
            ["clang", "-fplugin=evil.so", "a.c"],
            ["clang", "@opts.rsp", "a.c"],
            ["clang", "a.c", "other.c"],
            ["clang", "-save-temps", "a.c"],
        ]:
            with self.subTest(argv=argv):
                with self.assertRaises(ValueError):
                    clang.analysis_flags(argv, src, cwd)

    def test_clang_dependency_output_parses_escaped_paths(self):
        raw = 'excession: src.c include/with\\ space.h \\\n sys/header.h\n'
        self.assertEqual(
            clang.make_dependencies(raw),
            ["include/with space.h", "src.c", "sys/header.h"],
        )

    def test_ambiguous_compilation_database_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            (path / "unit.c").write_text("int x;")
            cmd = {"directory": str(path), "file": "unit.c",
                   "arguments": ["clang", "-c", "unit.c"]}
            (path / "compile_commands.json").write_text(json.dumps([cmd, cmd]))
            with self.assertRaisesRegex(ValueError, "exactly one"):
                clang.load_command(path / "compile_commands.json",
                                   (path / "unit.c").resolve())

    @unittest.skipUnless(shutil.which("clang++"), "clang++ unavailable")
    def test_clang_cpp_namespaces_methods_and_calls(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            content = (
                "namespace zoo {\\n"
                "int twice(int n) { return n + n; }\\n"
                "struct Pet { int feed() { return twice(2); } };\\n"
                "}\\n"
                "int main() { zoo::Pet p; return p.feed(); }\\n"
            )
            source = root / "pet.cc"
            source.write_text(content)
            blob = __import__("hashlib").sha1(
                b"blob " + str(len(content.encode())).encode() + b"\\0" + content.encode()
            ).hexdigest()
            db = root / "compile_commands.json"
            db.write_text(json.dumps([{
                "directory": str(root), "file": "pet.cc",
                "arguments": ["aarch64-linux-gnu-g++", "-std=c++17",
                              "-c", "pet.cc", "-o", "pet.o"],
            }]))
            request = {
                "trusted_local": True, "repo": str(root),
                "source_path": "pet.cc", "source_blob": blob,
                "compile_db": str(db), "compiler": "clang++", "config": None,
            }
            observation = clang.analyze(request)
            self.assertEqual(observation["language"], "c++")
            definitions = {s["qualified_name"] for s in observation["symbols"]
                           if s["definition"]}
            self.assertTrue({"zoo::twice", "zoo::Pet::feed", "main"}.issubset(definitions),
                            definitions)
            self.assertTrue(any(x["caller"] == "zoo::Pet::feed"
                                and x["callee"] == "twice"
                                for x in observation["calls"]), observation["calls"])
            self.assertTrue(any(x["caller"] == "main"
                                and x["callee"] == "feed"
                                for x in observation["calls"]), observation["calls"])
            self.assertEqual(observation["calls"], clang.analyze(request)["calls"])

    def test_arbitrary_cross_compiler_name_is_only_metadata(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "sample.c").write_text("int a;\\n")
            db = root / "compile_commands.json"
            db.write_text(json.dumps([{
                "directory": str(root), "file": "sample.c",
                "arguments": ["riscv64-linux-gnu-gcc", "-c", "sample.c"]
            }]))
            _, _, _, argv = clang.load_command(db, (root / "sample.c").resolve())
            self.assertEqual(argv[0], "riscv64-linux-gnu-gcc")

    @unittest.skipUnless(shutil.which("clang"), "clang unavailable")
    def test_clang_real_ast_and_header_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "sample.h").write_text("static inline int helper(int x) { return x+2; }\n")
            body = '#include "sample.h"\nint entry(int a) { return helper(a); }\n'
            source = root / "sample.c"
            source.write_text(body)
            blob = __import__("hashlib").sha1(
                b"blob " + str(len(body.encode())).encode() + b"\0" + body.encode()
            ).hexdigest()
            db = root / "compile_commands.json"
            db.write_text(json.dumps([{"directory": str(root), "file": "sample.c",
                                      "arguments": ["clang", "-c", "sample.c",
                                                    "-o", "sample.o"]}]))
            request = {"trusted_local": True, "repo": str(root),
                       "source_path": "sample.c", "source_blob": blob,
                       "compile_db": str(db), "compiler": "clang", "config": None}
            observed = clang.analyze(request)
            self.assertEqual(observed["status"],
                             "CLANG_AST_OBSERVED_NOT_BUILD_VERIFIED")
            self.assertTrue(any(s["name"] == "entry" and s["definition"]
                                for s in observed["symbols"]))
            self.assertTrue(any(c["caller"] == "entry" and c["callee"] == "helper"
                                for c in observed["calls"]))
            self.assertTrue(any(h["repo_path"] == "sample.h"
                                for h in observed["dependencies"]))
            self.assertEqual(observed["calls"], clang.analyze(request)["calls"])


if __name__ == "__main__":
    unittest.main()
