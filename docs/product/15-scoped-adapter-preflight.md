# Review/refactor: analyzer preflight must honor explicit roots

An explicitly scoped Python analysis was previously rejected if any unrelated JavaScript file elsewhere in the same repository was discovered and Node was unavailable. The global source inventory was incorrectly treated as the required adapter set.

`scoped_languages` now preserves the existing fast discovery counts when roots are automatic, and for explicit roots uses the **same Git commit** plus the existing `index::module_name` root-selection behavior to determine which adapters are actually needed. Only selected source modules require available analyzers. No file or language parser has been introduced, and no uncommitted files enter the inventory.

The real Git integration test places Python imports under `pkg/`, JS under `web/`, disables Node only for the child process, verifies `--roots pkg` still works, and verifies `--roots web` or unscoped discovery explicitly refuses the missing adapter.

The explicit-root path incurs an extra Git tree listing to preserve correct adapter decisions. Large-repository overhead remains a benchmarking question, rather than a premature homegrown filesystem index.
