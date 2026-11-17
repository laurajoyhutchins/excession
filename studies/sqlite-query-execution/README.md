# SQLite query preparation and execution

## Source identity

- Repository: `sqlite/sqlite`, a Git mirror of the canonical SQLite Fossil repository.
- Git commit: `3d0c703e3d18b05099d90163189bca51e6820526`
- Git tree: `789c8e8efeb78e14df60c2291228755f1efaba6c`
- Fossil check-in (`manifest.uuid`): `785291512e764fe0fa7e18d2be0b1ff53740a068af494b08727ea75cdab9a9ce`
- Code question: How does SQL text become a VDBE program, and how does `sqlite3_step()` execute it?

## Independent source-reading baseline

These are manually checked **source call sites**, not claims recovered by Excession or a full dynamic trace. Each is anchored to the pinned source revision.

| Stage | Source | Observation |
| --- | --- | --- |
| Public preparation API | [src/prepare.c:972](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/prepare.c#L972) | `sqlite3_prepare_v2` calls `sqlite3LockAndPrepare` with saved-SQL preparation. |
| Preparation | [src/prepare.c:871](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/prepare.c#L871) | `sqlite3LockAndPrepare` enters schema/B-tree locks and retries preparation for certain schema errors. |
| Parse entry | [src/prepare.c:814](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/prepare.c#L814) | `sqlite3Prepare` calls `sqlite3RunParser`. |
| Tokenization and grammar | [src/tokenize.c:600](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/tokenize.c#L600), [src/tokenize.c:713](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/tokenize.c#L713) | `sqlite3RunParser` tokenizes SQL and feeds the Lemon-generated `sqlite3Parser`. |
| SELECT dispatch | [src/parse.y:520](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/parse.y#L520) | The SELECT command grammar action calls `sqlite3Select`. |
| Query compilation | [src/select.c:7928](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/select.c#L7928), [src/select.c:8653](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/select.c#L8653) | `sqlite3Select` can request `sqlite3WhereBegin` and later `sqlite3WhereEnd` to generate scan loops. |
| Loop/code generation | [src/where.c:6930](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/where.c#L6930), [src/where.c:7001](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/where.c#L7001) | The WHERE machinery chooses access paths and emits VDBE loop/cursor instructions. |
| Opcode storage | [src/vdbeaux.c:272](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/vdbeaux.c#L272) | `sqlite3VdbeAddOp3` appends an opcode and operands to the VDBE instruction array. |
| Program finalization | [src/build.c:152](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/build.c#L152), [src/build.c:284](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/build.c#L284) | `sqlite3FinishCoding` calls `sqlite3VdbeMakeReady`. |
| Runtime API | [src/vdbeapi.c:990](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/vdbeapi.c#L990), [src/vdbeapi.c:941](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/vdbeapi.c#L941) | `sqlite3_step` calls `sqlite3Step`, which invokes `sqlite3VdbeExec`. |
| Interpreter | [src/vdbe.c:901](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/vdbe.c#L901), [src/vdbe.c:1807](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/vdbe.c#L1807) | `sqlite3VdbeExec` dispatches VDBE opcodes, including `OP_ResultRow`. |
| Schema recovery | [src/vdbeapi.c:990](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/vdbeapi.c#L990) | The public step wrapper can reprepare the statement on a schema invalidation. |

### Provisional architectural interpretation

SQLite's SELECT compiler is also a bytecode generator. It does not merely hand an abstract plan to a separate execution planner. In `sqlite3WhereBegin`/`sqlite3WhereEnd`, planning and code emission are closely connected. The resulting VDBE instructions execute through `sqlite3VdbeExec`. The execution API also owns an important invalidation/reprepare boundary.

This does **not** establish full resolved call-graph completeness, every configuration's behavior, or correctness of the generated code. It describes one source-backed route through a normal SELECT and selected recovery behavior.

## Actual Excession experiment

Excession's existing owner-authored GitHub-native workflow was enabled for this read-only public repository through [PR #273](https://github.com/laurajoyhutchins/excession/pull/273), with no untrusted source execution.

- [Issue #274: `agent locate`](https://github.com/laurajoyhutchins/excession/issues/274), [Actions run 44009760787](https://github.com/laurajoyhutchins/excession/actions/runs/44009760787). The task is to locate SQL preparation, parsing, planning, bytecode generation, and VDBE execution.
- [Issue #275: `agent context`](https://github.com/laurajoyhutchins/excession/issues/275), [Actions run 44009846141](https://github.com/laurajoyhutchins/excession/actions/runs/44009846141). The focus is the nine relevant SQLite source/grammar files.

Inspect the **original** GitHub artifacts `report.json` and `receipt.json`; confirm `COMPLETED`, exact commit and tree, and Excession's declared coverage before drawing conclusions. No Excession output is represented by this baseline.

## Measured Excession results (original pilot)

Both GitHub-native runs completed successfully, and the artifact `receipt.json` files declare the exact Git commit/tree and `COMPLETED`. The `report.json` SHA-256 digests were independently recomputed and matched the receipts.

- **Locate:** [run 44009760787](https://github.com/laurajoyhutchins/excession/actions/runs/44009760787), report SHA-256 `63dce81e1df6b89a2e9629110166c6aa761b21ed204204c5dae4d558ffb65513`. Excession indexed **481 of 494** eligible source files (12,102,161 bytes) and skipped **13**. It returned 24 candidates and found **4 of the 9** independently selected reference files in the first 24: `prepare.c` (#1), `vdbeaux.c` (#3), `build.c` (#7), and `vdbeapi.c` (#16). It additionally located the important `wherecode.c` (#12). The index reported `complete=false` even with zero pending segments because of skipped files; candidate excerpts were lexical heuristics.
- **Context:** [run 44009846141](https://github.com/laurajoyhutchins/excession/actions/runs/44009846141), report SHA-256 `4de81fdae46cf6a54e0c3e3a44bcda26e3a840e800e5b62276c2ab3900cb8eb8`. All 9 requested source paths were authenticated to Git, but the report had **zero** source relationships, zero compiler reference observations, and zero configured observations. It explicitly reports **359 C source files without a supported static-module producer**, plus additional unrelated language gaps. Its 60 test suggestions are path-name heuristics, not validated test choices.

The critical retrieval failure has a concrete mechanical cause. The original locator capped a single source at 256 KiB, excluding:

| Essential source | Bytes | Stage |
| --- | ---: | --- |
| `src/select.c` | 347,619 | SELECT compilation |
| `src/where.c` | 306,736 | Plan selection and loop generation |
| `src/vdbe.c` | 328,620 | VM execution |

The source `src/wherecode.c` (125,043 bytes) contains `sqlite3WhereCodeOneLoopStart` ([line 1649](https://github.com/sqlite/sqlite/blob/3d0c703e3d18b05099d90163189bca51e6820526/src/wherecode.c#L1649)) and is therefore part of the independent execution-path baseline even though it was outside the original nine-file focus list.

**Conclusion:** source identity and reproducibility succeeded. Unconstrained architectural discovery did not: the first run missed essential implementation files and the second supplied no resolved C-source graph. This is a negative, reproducible result, not evidence that SQLite lacks those relationships.

A targeted improvement is tracked in [PR #277](https://github.com/laurajoyhutchins/excession/pull/277): enlarge bounded lexical indexing and invalidate stale exclusion decisions as part of segment identity. Even after that correction, **native C symbol and configured-build observations remain an independent obligation**. Do not equate improved keyword retrieval with a verified execution path.

## Evaluation

For the `locate` report, measure where the nine reference paths rank, relevant source windows returned, misses, false leads, and index completeness.

For the `context` report, distinguish native compiler-observed references, lexical/static candidates, and missing C-language semantic coverage. An empty edge set is not evidence that the source files are unrelated.

The first improvement to Excession should target an observed failure or missing capability in these reports rather than implementing a new speculative analysis pipeline.
