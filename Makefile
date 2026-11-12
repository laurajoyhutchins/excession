.PHONY: check check-full build rust-check rust-full-check python-check node-check go-check rust-junit python-junit junit cli install-cli benchmark-schema-check

build:
	cargo build --offline --locked --manifest-path core/Cargo.toml

rust-check:
	cargo fmt --manifest-path core/Cargo.toml -- --check
	cargo clippy --offline --locked --manifest-path core/Cargo.toml --all-targets -- -D warnings
	cargo test --offline --locked --manifest-path core/Cargo.toml --lib

# Full Rust integration tests exercise optional language/native adapters.
rust-full-check:
	cargo test --offline --locked --manifest-path core/Cargo.toml

python-check: build
	python3 -m unittest discover -s tests -v

node-check:
	npm run check
	npm test

go-check:
	GO111MODULE=off go test ./core/adapters

check: rust-check node-check

# Opt-in Python-specific integration and independent-oracle checks.
check-full: check rust-full-check python-check

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
