# Historical pytest behavioral oracle for ontology proposals

This is an **integration and negative-control experiment**, not a claimed novel architectural discovery.

The experiment downloads the exact upstream historical faulty commit
`bc14038c406563b18a03e15453a10b9eee969e42` and fixed commit
`dcd0e636b4ce69021a9f607b060ea0056c339fd2` from
[pytest-dev/pytest](https://github.com/pytest-dev/pytest).
The independent method-level probe checks that
`Collector.repr_failure(Collector.CollectError())` raises an `IndexError`
in the faulty commit and returns an empty string in the fixed commit.

Then it constructs **one controlled source-only regression** on top of the
fixed commit, restoring the upstream faulty expression without modifying
`testing/test_nodes.py`. The same probe must fail with the original exception.

The source-companion learner is trained only on the pinned fixed revision's
earlier Git history and emits the selected policy's **specific unmodified test
prediction**. This is compared with global file popularity, same-directory
popularity, normalized file-name counterpart, and `pytest-testmon==2.3.0`,
which receives a real prewarmed executed-test baseline. The native selector
**must** catch the regression; this control cannot be silently disabled.

The output exposes every predictor result, independently executed logs, exact
commit and tree identities, and SHA-256 log/report digests. Missing or
unexecutable behavioral evidence fails the workflow.

## Interpretation boundaries

This experiment establishes the precise semantics of connecting an
ontology-source proposal to a real runnable test. It does not prove a new
semantic law or that Excession exceeds native tooling. In particular, the
first historical faulty commit is already defective; we have not located its
actual upstream introduction in an A-pass/B-fail/C-pass sequence. The new
controlled commit after the fix is a seeded change, **not an independently
observed coding-agent mistake**.

Once this control is functioning, the next study needs previously unseen,
independently adjudicated A/B/C triplets and an ontology frozen before B.
The test-oracle or native-baseline outcome must not be used to tune the model.
