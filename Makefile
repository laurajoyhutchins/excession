.PHONY: check rust-check python-check build cli install-cli
build:
	cargo build --offline --manifest-path core/Cargo.toml
rust-check:
	cargo fmt --manifest-path core/Cargo.toml -- --check
	cargo clippy --offline --manifest-path core/Cargo.toml --all-targets -- -D warnings
	cargo test --offline --manifest-path core/Cargo.toml
python-check: build
	python3 -m unittest discover -s tests -v
check: rust-check python-check

cli:
	cargo build --offline --manifest-path core/Cargo.toml --bin excession
install-cli:
	cargo install --offline --locked --path core --bin excession
