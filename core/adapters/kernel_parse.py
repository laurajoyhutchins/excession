"""Conservative Linux C/Kbuild/Kconfig *syntax* extractors.

Input is a JSON array of {path, content} records on stdin. Output is an
equally ordered array of {path, facts, diagnostics}. No target code, make
recipes, or Kconfig expressions are executed. Unresolved conditions remain
source text rather than being guessed true or false.
"""
import json
import re
import sys

DIRECTIVE = re.compile(r"^\s*#\s*(\w+)\b(.*)$")
INCLUDE = re.compile(r'^\s*(?:"([^"\n]+)"|<([^>\n]+)>)\s*$')
EXPORT = re.compile(r"\b(EXPORT_SYMBOL(?:_GPL|_NS|_NS_GPL)?)\s*\(\s*([a-zA-Z_]\w*)\s*(?:,\s*([a-zA-Z_]\w*))?\s*\)")
ASSIGN = re.compile(r"^\s*([^\s:=+?]+)\s*([:+?]?=)\s*(.*?)\s*$")
KCONFIG_SYMBOL = re.compile(r"^\s*(?:menuconfig|config)\s+([A-Za-z_][A-Za-z_0-9]*)\s*$")
CONFIG_REF = re.compile(r"^\$\(CONFIG_([A-Za-z_0-9]+)\)$")


def logical_lines(text):
    """Track original starting line for backslash-newline continuations."""
    buf = ""
    start = 1
    for number, line in enumerate(text.splitlines(), 1):
        if not buf:
            start = number
        piece = line.rstrip()
        if piece.endswith("\\"):
            buf += piece[:-1] + " "
        else:
            yield start, buf + piece
            buf = ""
    if buf:
        yield start, buf


def without_c_comments(text):
    """Mask comments without touching quoted strings or physical line counts."""
    out = []
    state = "code"
    i = 0
    while i < len(text):
        c = text[i]
        n = text[i + 1] if i + 1 < len(text) else ""
        if state == "code":
            if c == "/" and n == "/":
                state = "line_comment"
                out.extend("  ")
                i += 2
                continue
            if c == "/" and n == "*":
                state = "block_comment"
                out.extend("  ")
                i += 2
                continue
            if c in ('"', "'"):
                state = c
            out.append(c)
        elif state == "line_comment":
            if c == "\n":
                state = "code"
            out.append("\n" if c == "\n" else " ")
        elif state == "block_comment":
            if c == "*" and n == "/":
                out.extend("  ")
                state = "code"
                i += 2
                continue
            out.append("\n" if c == "\n" else " ")
        else:
            out.append(c)
            if c == "\\" and n:
                out.append(n)
                i += 2
                continue
            if c == state:
                state = "code"
        i += 1
    return "".join(out)


def mask_c_literals(line):
    """Mask C string/character literal interiors before scanning macro calls."""
    result = []
    quote = ""
    escaped = False
    for char in line:
        if quote:
            result.append(" ")
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == quote:
                quote = ""
        elif char in ('"', "'"):
            quote = char
            result.append(" ")
        else:
            result.append(char)
    return "".join(result)


def fact(line, kind, subject, target="", condition="", status="DECLARED"):
    return dict(line=line, kind=kind, subject=subject, target=target,
                condition=condition, status=status)


def cond(stack):
    return " && ".join("("+x+")" for x in stack if x)


def c_facts(text):
    facts = []
    diagnostics = []
    stack = []
    # Kept for #elif/#else; do not evaluate preprocessor conditions.
    branches = []
    masked = without_c_comments(text)
    for line, source in logical_lines(masked):
        m = DIRECTIVE.match(source)
        if not m:
            for exported in EXPORT.finditer(mask_c_literals(source)):
                name, symbol, namespace = exported.groups()
                facts.append(fact(line, "c_export", symbol, namespace or "",
                                  cond(stack), "LEXICAL_CANDIDATE"))
            continue
        directive, rest = m.group(1), m.group(2).strip()
        if directive in ("if", "ifdef", "ifndef"):
            expression = (f"defined({rest})" if directive == "ifdef" else
                          f"!defined({rest})" if directive == "ifndef" else rest)
            stack.append(expression)
            branches.append([expression])
        elif directive == "elif":
            if not stack:
                diagnostics.append(dict(line=line, reason="unmatched #elif"))
            else:
                seen = branches[-1]
                stack[-1] = f"!({' || '.join(seen)}) && ({rest})"
                seen.append(rest)
        elif directive == "else":
            if not stack:
                diagnostics.append(dict(line=line, reason="unmatched #else"))
            else:
                stack[-1] = f"!({' || '.join(branches[-1])})"
        elif directive == "endif":
            if not stack:
                diagnostics.append(dict(line=line, reason="unmatched #endif"))
            else:
                stack.pop()
                branches.pop()
        elif directive == "include":
            value = INCLUDE.match(rest)
            if value:
                quoted, angled = value.groups()
                facts.append(fact(line, "c_include",
                                  "quoted" if quoted else "system",
                                  quoted or angled, cond(stack), "UNRESOLVED_INCLUDE"))
            else:
                diagnostics.append(dict(line=line, reason="computed #include: " + rest[:150]))
        elif directive == "define":
            # Macro *declarations*, never claim that expansions occurred.
            word = rest.split(None, 1)[0] if rest else ""
            if re.fullmatch(r"[A-Za-z_]\w*", word):
                facts.append(fact(line, "c_macro", word, "", cond(stack),
                                  "LEXICAL_CANDIDATE"))
    if stack:
        diagnostics.append(dict(line=len(text.splitlines()) or 1,
                                reason="unterminated C preprocessor condition"))
    return facts, diagnostics


def build_facts(text):
    facts = []
    diagnostics = []
    condition_stack = []
    for line, source in logical_lines(text):
        source = source.split("#", 1)[0].strip()
        if not source:
            continue
        if source.startswith(("ifeq ", "ifneq ", "ifdef ", "ifndef ")):
            condition_stack.append(source)
            continue
        if source == "else":
            if condition_stack:
                condition_stack[-1] = "not(" + condition_stack[-1] + ")"
            else:
                diagnostics.append(dict(line=line, reason="unmatched make else"))
            continue
        if source == "endif":
            if condition_stack:
                condition_stack.pop()
            else:
                diagnostics.append(dict(line=line, reason="unmatched make endif"))
            continue
        if source.startswith("include ") or source.startswith("-include "):
            facts.append(fact(line, "build_include", "make", source.split(None, 1)[1],
                              cond(condition_stack), "UNRESOLVED_MAKE"))
            continue
        m = ASSIGN.match(source)
        if not m:
            continue
        variable, op, value = m.groups()
        if not (variable.startswith("obj-") or variable.endswith(("-y", "-m", "-objs")) or
                "-$(CONFIG_" in variable):
            continue
        guard = ""
        if "-$(CONFIG_" in variable:
            prefix, rest = variable.split("-$(CONFIG_", 1)
            if rest.endswith(")"):
                guard = "CONFIG_" + rest[:-1]
                variable = prefix
        elif variable.endswith("-m"):
            guard = "MODULE"
            variable = variable[:-2]
        elif variable.endswith("-y"):
            guard = "BUILTIN"
            variable = variable[:-2]
        elif variable.endswith("-objs"):
            guard = "COMPOSITE"
            variable = variable[:-5]
        if "$(" in value or "${" in value:
            diagnostics.append(dict(line=line, reason="dynamic make RHS: " + value[:150]))
        for target in value.split():
            if target.endswith((".o", "/")) and "$" not in target:
                facts.append(fact(line, "build_object", variable, target,
                                  cond(condition_stack + [guard]), "UNRESOLVED_CONFIG"))
        if op not in ("+=", ":=", "="):
            diagnostics.append(dict(line=line, reason="unusual make assignment: " + op))
    if condition_stack:
        diagnostics.append(dict(line=len(text.splitlines()) or 1,
                                reason="unterminated make conditional"))
    return facts, diagnostics


def kconfig_facts(text):
    facts = []
    diagnostics = []
    symbol = ""
    stack = []
    help_indent = None
    for line, raw in logical_lines(text):
        source = raw.strip()
        indent = len(raw.expandtabs(8)) - len(raw.expandtabs(8).lstrip())
        if help_indent is not None:
            if not source or indent > help_indent:
                continue
            help_indent = None
        if not source or source.startswith("#"):
            continue
        if source in ("help", "---help---"):
            help_indent = indent
            continue
        m = KCONFIG_SYMBOL.match(source)
        if m:
            symbol = m.group(1)
            facts.append(fact(line, "kconfig_symbol", symbol, "", cond(stack)))
            continue
        if source.startswith("if "):
            stack.append(source[3:].strip())
            continue
        if source == "endif":
            if stack:
                stack.pop()
            else:
                diagnostics.append(dict(line=line, reason="unmatched Kconfig endif"))
            continue
        if source.startswith(("source ", "rsource ", "osource ", "orsource ")):
            kw, path = source.split(None, 1)
            facts.append(fact(line, "kconfig_source", kw, path.strip("'\""),
                              cond(stack), "UNRESOLVED_SOURCE"))
            continue
        if not symbol:
            continue
        if source.startswith("depends on "):
            facts.append(fact(line, "kconfig_depends", symbol, source[11:].strip(),
                              cond(stack), "UNRESOLVED_CONFIG"))
        elif source.startswith(("select ", "imply ")):
            kw, rest = source.split(None, 1)
            target, _, guard = rest.partition(" if ")
            facts.append(fact(line, "kconfig_" + kw, symbol, target.split()[0],
                              cond(stack + ([guard] if guard else [])), "UNRESOLVED_CONFIG"))
        elif source.startswith("default "):
            facts.append(fact(line, "kconfig_default", symbol, source[8:].strip(),
                              cond(stack), "UNRESOLVED_CONFIG"))
        elif source in ("endmenu", "endchoice"):
            symbol = ""
    if stack:
        diagnostics.append(dict(line=len(text.splitlines()) or 1,
                                reason="unterminated Kconfig if"))
    return facts, diagnostics


def parse(path, content):
    basename = path.rsplit("/", 1)[-1]
    if path.endswith((".c", ".h")):
        facts, errors = c_facts(content)
    elif basename in ("Makefile", "Kbuild"):
        facts, errors = build_facts(content)
    elif basename.startswith("Kconfig"):
        facts, errors = kconfig_facts(content)
    else:
        raise ValueError("unsupported kernel source type")
    return dict(path=path, facts=facts, diagnostics=errors)


def main():
    items = json.load(sys.stdin)
    if not isinstance(items, list) or len(items) > 256:
        raise ValueError("expected at most 256 source records")
    outputs = []
    for item in items:
        if not isinstance(item, dict) or set(item) != {"path", "content"}:
            raise ValueError("bad source record")
        if not isinstance(item["path"], str) or not isinstance(item["content"], str):
            raise ValueError("non-text source record")
        outputs.append(parse(item["path"], item["content"]))
    print(json.dumps(outputs, sort_keys=True, separators=(",", ":"), ensure_ascii=False))


if __name__ == "__main__":
    main()
