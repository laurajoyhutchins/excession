.PHONY: check build rust-check python-check node-check go-check rust-junit python-junit junit cli install-cli benchmark-schema-check native-preflight grammar-verify

build:
	cargo build --offline --locked --manifest-path core/Cargo.toml

rust-check:
	cargo fmt --manifest-path core/Cargo.toml -- --check
	cargo clippy --offline --locked --manifest-path core/Cargo.toml --all-targets -- -D warnings
	cargo test --offline --locked --manifest-path core/Cargo.toml

python-check: build
	python3 -m unittest discover -s tests -v

node-check:
	npm run check
	npm test

go-check:
	GO111MODULE=off go test ./core/adapters

check: rust-check python-check node-check

# Optional JUnit reports; requires cargo-nextest and pytest.
rust-junit:
	cargo nextest run --offline --locked --manifest-path core/Cargo.toml --profile ci
	cargo test --offline --locked --manifest-path core/Cargo.toml --doc

python-junit: build
	python3 -m pytest -q tests --junitxml=test-results/python.xml

junit: rust-junit python-junit

benchmark-schema-check:
	cargo test --offline --locked --manifest-path core/Cargo.toml benchmark::tests

cli:
	cargo build --offline --locked --manifest-path core/Cargo.toml --bin excession

install-cli:
	cargo install --offline --locked --path core --bin excession

native-preflight:
	python3 tools/native_runtime.py

grammar-verify:
	python3 tools/grammar_artifacts.py verify
