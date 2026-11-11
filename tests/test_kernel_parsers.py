"""Regression tests for conservative native Linux syntax extraction."""
import importlib.util
import unittest
from pathlib import Path

MODULE = Path(__file__).resolve().parents[1] / "core/adapters/kernel_parse.py"
spec = importlib.util.spec_from_file_location("kernel_parse", MODULE)
parser = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parser)


class KernelParsers(unittest.TestCase):
    def test_c_preprocessor_guard_and_comments(self):
        source = """/* #include <fake.h> */
#define SOME_MACRO 1
#include <linux/kernel.h>
#if defined(CONFIG_ALPHA)
#include "alpha.h"
#elif defined(CONFIG_BETA)
#include <beta.h>
#else
#include HDR
#endif
EXPORT_SYMBOL_GPL(public_symbol);
"""
        facts, diagnostics = parser.c_facts(source)
        includes = [f for f in facts if f["kind"] == "c_include"]
        self.assertEqual([i["target"] for i in includes],
                         ["linux/kernel.h", "alpha.h", "beta.h"])
        self.assertEqual(includes[1]["condition"], "(defined(CONFIG_ALPHA))")
        self.assertIn("CONFIG_BETA", includes[2]["condition"])
        self.assertTrue(any("computed #include" in d["reason"] for d in diagnostics))
        self.assertTrue(any(f["subject"] == "public_symbol" and
                            f["kind"] == "c_export" for f in facts))

    def test_c_export_text_inside_strings_is_not_a_symbol(self):
        text = '''const char *example = "EXPORT_SYMBOL(false_symbol)";
EXPORT_SYMBOL(real_symbol);
'''
        facts, _ = parser.c_facts(text)
        exports = [f["subject"] for f in facts if f["kind"] == "c_export"]
        self.assertEqual(exports, ["real_symbol"])

    def test_kconfig_help_text_is_not_a_config_statement(self):
        text = '''config FOO
    bool "example"
    help
      if FAKE
      depends on FAKE
config BAR
    depends on FOO
'''
        facts, diagnostics = parser.kconfig_facts(text)
        self.assertEqual(diagnostics, [])
        self.assertEqual(
            [f["target"] for f in facts if f["kind"] == "kconfig_depends"],
            ["FOO"],
        )

    def test_c_preprocessor_rejects_unmatched_blocks(self):
        _, diagnostics = parser.c_facts("#else\n#if FOO\n#include <x.h>\n")
        self.assertEqual(len(diagnostics), 2)
        self.assertTrue(any("unterminated" in d["reason"] for d in diagnostics))

    def test_kbuild_object_and_config_condition(self):
        text = """obj-$(CONFIG_RTC_DRV_CMOS) += rtc-cmos.o
rtc-core-y := class.o interface.o
rtc-core-$(CONFIG_RTC_NVMEM) += nvmem.o
obj-$(CONFIG_EXAMPLE) += $(wildcard foo*.o)
ifeq ($(CONFIG_X),y)
obj-y += shared.o
endif
"""
        facts, diagnostics = parser.build_facts(text)
        pair = next(f for f in facts if f["target"] == "rtc-cmos.o")
        self.assertEqual(pair["subject"], "obj")
        self.assertIn("CONFIG_RTC_DRV_CMOS", pair["condition"])
        self.assertEqual({f["target"] for f in facts if f["subject"] == "rtc-core"},
                         {"class.o", "interface.o", "nvmem.o"})
        self.assertTrue(any("dynamic make RHS" in d["reason"] for d in diagnostics))

    def test_kconfig_declaration_and_unresolved_select(self):
        text = """config RTC_LIB
    bool
menuconfig RTC_CLASS
    bool "RTC"
    depends on !S390
    select RTC_LIB
if RTC_CLASS
config RTC_DRV_CMOS
    tristate "CMOS"
    depends on X86 || ARM
    select RTC_MC146818_LIB if X86
endif
source "drivers/rtc/Kconfig"
"""
        facts, diagnostics = parser.kconfig_facts(text)
        self.assertEqual(diagnostics, [])
        symbols = [f["subject"] for f in facts if f["kind"] == "kconfig_symbol"]
        self.assertEqual(symbols, ["RTC_LIB", "RTC_CLASS", "RTC_DRV_CMOS"])
        select = next(f for f in facts if f["kind"] == "kconfig_select" and
                      f["subject"] == "RTC_DRV_CMOS")
        self.assertEqual(select["target"], "RTC_MC146818_LIB")
        self.assertIn("X86", select["condition"])
        self.assertIn("RTC_CLASS", select["condition"])

    def test_kconfig_source_refuses_to_infer_existence(self):
        facts, _ = parser.kconfig_facts('source "$srctree/arch/Kconfig"')
        self.assertEqual(facts[0]["status"], "UNRESOLVED_SOURCE")

    def test_parser_refuses_unsupported_file(self):
        with self.assertRaises(ValueError):
            parser.parse("README.md", "not a source file")


if __name__ == "__main__":
    unittest.main()
