.PHONY: check rust-check python-check build
build:
	cargo build --offline --manifest-path core/Cargo.toml
rust-check:
	cargo fmt --manifest-path core/Cargo.toml -- --check
	cargo clippy --offline --manifest-path core/Cargo.toml --all-targets -- -D warnings
	cargo test --offline --manifest-path core/Cargo.toml
python-check: build
	python3 -m unittest discover -s tests -v
check: rust-check python-check
