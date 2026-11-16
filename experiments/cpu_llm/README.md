# CPU code-retrieval experiment

Status: **research only, do not enable by default**. This is not a
verified semantic ontology system or a test-elision authority.

## Reproduce

```sh
python -m pip install 'numpy==2.3.5' 'onnxruntime==1.23.2' \
  'tokenizers==0.22.1' 'huggingface_hub==1.16.1' 'rank-bm25==0.2.2'
python experiments/cpu_llm/benchmark_minilm.py
```

The harness pins all source commit IDs and Git blob IDs, verifies source bytes
with the Git blob hash, and pins the MiniLM ONNX model revision and SHA-256
of model and tokenizer. Results are written to `cpu_llm_results/minilm.json`.

* Model: `Xenova/ms-marco-MiniLM-L-6-v2`
* Model revision: `a09144355adeed5f58c8ed011d209bf8ee5a1fec`
* Weights: `onnx/model_quantized.onnx`, SHA-256 `e9d8ebf845c413e981c175bfe49a3bfa9b3dcce2a3ba54875ee5df5a58639fbe`
* Tokenizer SHA-256: `d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66`
* Runtime: ONNX Runtime 1.23.2 CPU, 4 inference threads, 256 total tokens per query/passage pair.

## Corpus and design

17 real Git-blob-verified source files: Excession (4), h11 (5),
Kubernetes (4), and Linux (4). Each repo has human-authored preliminary
source-intent questions, including explicit no-match cases. Excession and
h11 form the development split (13 questions); Kubernetes and Linux form
the held-out split (12 questions). Results additionally include four
synthetic adversarial comment insertions. The inserted comments **are
not** part of the pinned Git tree.

The questions and labels were written for this experiment; **they have
not undergone independent code-owner review**. There are only eight
positive held-out tasks, far too few for statistically confident product
conclusions. Selecting or changing a model, prompt, snippets or scoring
method after inspecting the held-out results also means the same held-out
set can no longer be treated as an unbiased final test.

The measured default is *code-window first*, then source signatures
and a bounded file header. Input is 256 tokens with truncation. Its
predecessor was header-first and produced an invalid attack comparison
because the injected text was truncated away.

## Latest code-window-first run

[Workflow run 43617052705](https://github.com/laurajoyhutchins/excession/actions/runs/43617052705),
result artifact `excession-minilm-cpu-results`:

| Metric | MiniLM ONNX INT8 | BM25 | Token overlap |
| --- | ---: | ---: | ---: |
| Development positive top-1 | 5/9 | 4/9 | 3/9 |
| Development full task correct (incl. no-match) | 9/13 | 8/13 | 5/13 |
| Held-out positive top-1 | 7/8 | **8/8** | 7/8 |
| Held-out full task correct (incl. no-match) | 8/12 | **9/12** | 7/12 |
| Held-out median inference | **32.7 ms** | no neural inference | no neural inference |
| Held-out inference p95 | **37.3 ms** | no neural inference | no neural inference |

The development-only threshold was fitted separately for each scoring
system, then evaluated unchanged on held-out repositories. MiniLM's
threshold still transferred imperfectly. Process peak RSS was
approximately **253 MB**, including Python and ONNX runtime. All
measurements are from one four-vCPU hosted runner, not a developer-laptop p95.

### Adversarial input

The corrected attack puts a fake task-matching source comment **before**
the implementation, inside the model's token budget. The attacked candidate's
MiniLM logit rose substantially in all four cases (examples: Excession
from about -11.3 to +8.8; h11 from -11.4 to +7.7). Three of four tests
changed the highest-ranked file. This establishes susceptibility to
adversarially written comments that bias semantic relevance. It does
**not** by itself establish that the reranker executes instructions.

The original header-first test observed 8/8 held-out correct positive
rankings, but just 4/12 full task accuracy after abstention. That
attack test was invalid because the comment was outside the input
window. Do not cherry-pick that earlier result.

## Engineering recommendation

Do not route untrusted code comments into a semantic authority path.
SCIP, Git, parser spans, deterministic symbol relationships and existing
test selectors must remain the authority. Use BM25 or native relationship
retrieval first; add a reranker only if it improves independently
reviewed retrieval on truly unseen repositories without causing
unsafe no-match errors. Any neural rank must remain a *candidate*.

A follow-on acceptance test needs:
1. independently adjudicated labels, with exact Git revisions;
2. substantially more codebases and hard same-repository negatives;
3. strict train/development/test separation, with no tuning on final test;
4. explicit no-match calibration, comment/bait testing, and source-span
   provenance checks;
5. latency/memory distributions across representative developer CPUs;
6. a statistically meaningful improvement over native relationship
   retrieval and BM25 before default enablement.

See `benchmark_cpu_llms.py`, `benchmark_binary.py`,
`benchmark_choice.py`, `benchmark_reranker.py`, and
`benchmark_names.py` for earlier negative-result experiments.
