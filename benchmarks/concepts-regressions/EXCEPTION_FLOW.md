# Prospective exception-path obligations (development pilot)

This prototype turns a **source-only observation at working revision A** into a
falsifiable exception-propagation hypothesis. It is not a complete Java
control-flow analysis, proof of API compatibility, or automatically generated
behavioral counterexample.

## Algorithm

The Java compiler's official `com.sun.source.tree` API parses tracked
`src/main/java/**/*.java` files without compiling or executing project
source. The bounded adapter summarizes each Java catch body's possible
syntactic exits as NORMAL / THROW / RETURN, recognizes direct Java throws,
and follows a small set of unambiguous same-class helper calls. It carries
non-null exceptional values into those helpers and conservatively treats
unknown forms as possibly returning normally.

Rust `concepts exception-freeze` requires a **complete tracked-source scan**
from Git at A, binds every observation to its exact source blob, and freezes
catch handlers with a modeled always-throwing exit. The frozen report
contains neither B source content, C, nor the hidden oracle.

Only afterward can the Rust `concepts exception-assess` compare a separately
complete B-side scan. For an A catch handler in a modified file it reports:

- `PRESERVED_MODELED_THROW_GUARANTEE`: the same handler is still modeled as throwing;
- `LOST_MODELED_THROW_GUARANTEE`: the formerly throwing handler can now return;
- `HANDLER_REMOVED_OR_REKEYED_UNSUPPORTED`: comparison is not semantically supported.

Each entry carries A and B Git blob IDs, class/method/catch identities,
lines, and a syntax-only baseline: whether the same catch body lost its
last *direct* `throw` statement. That baseline is a deliberately simple
comparator, not CodeQL, a mature compiler dataflow engine, or a fair
bug-prevention benchmark. `model_only_handler_warnings` counts handler
relationships that only the limited interprocedural model flagged.

## Existing-dataset experiment

`.github/workflows/concepts-exception-flow.yml` uses the **three real
RegMiner A/B/C cases already verified** by the Maven oracle workflow.
Each case starts with an A-only shallow checkout, records an A scan and
frozen report, then fetches B and checks its immediate parent. It refuses
if the repair commit C was fetched into the candidate checkout. The
previously executed, independent RegMiner TestNG receipts are used only
to interpret the results afterward.

The `Github_24` known regression is especially instructive: the parser
originally propagated exceptions through `handleException`; B changed
the helper to construct an exception without reliably throwing it. C
restored propagation through `stopParsing` instead of merely reverting
the earlier source line. The AST analysis should identify the A→B loss
and can independently identify C's syntactic recovery in controlled
fixtures. This example was inspected during development. It is therefore
**not** a blind generalization result.

The A/B/C fixtures in `fixtures/exception-flow` also include an unchanged
logging-only catch as a negative control. They test the analysis semantics,
not the historical RegMiner oracle.

## Contract and limits

The analysis uses the standard JDK parser rather than regex but does
**not** resolve Java types or external method bodies. Ambiguous overloaded
helpers are unsupported; loops and unrecognized control flow are treated
as potentially normal. The producer is not cryptographically attested;
Rust binds its output to Git identities and validates full file coverage
but does not prove the producer reported every catch correctly.

Every result remains `UNVERIFIED_SOURCE_ONLY_CONTROL_FLOW_CHANGE`.
Loss of the modeled guarantee is a reason to investigate and generate
a counterexample, not a verified program defect. A later experiment must
generate an executable input from A-only evidence, demonstrate A-pass/
B-fail/C-pass without using a test introduced at C for generation,
and beat strong independent test-selection and static-analysis baselines
on a new held-out multi-repository cohort.

## CLI

```sh
excession concepts exception-freeze --repo upstream --rev "$A" --observations a.jsonl > frozen.json
excession concepts exception-assess --repo upstream --candidate "$B" \
  --frozen frozen.json --observations b.jsonl > review.json
```
