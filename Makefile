# One entry point for both toolchains. `make check` builds, lints and tests everything.
.NOTPARALLEL:

GO_MODULES := cli sdk-go renderers/tui protocol/go

.PHONY: check install build test test-ts test-go test-rust lint lint-ts lint-go lint-rust typecheck format format-check codegen codegen-check

check: install codegen-check format-check lint typecheck build test

install:
	bun install --frozen-lockfile

build: bin/suite

bin/suite: FORCE
	go build -o bin/suite ./cli

FORCE:

test: test-ts test-go test-rust

test-ts:
	bun test

test-go:
	@for m in $(GO_MODULES); do echo "go test $$m"; go -C $$m test ./... || exit 1; done

lint: lint-ts lint-go lint-rust

lint-ts:
	bunx oxlint --type-aware --deny-warnings

lint-go:
	@out=$$(gofmt -l $(GO_MODULES)); if [ -n "$$out" ]; then echo "gofmt needed:"; echo "$$out"; exit 1; fi
	@for m in $(GO_MODULES); do go -C $$m vet ./... || exit 1; done

test-rust:
	cargo test --workspace --quiet

lint-rust:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --quiet -- -D warnings

typecheck:
	bunx tsc -p tsconfig.json --noEmit

format:
	bunx prettier --write .
	gofmt -w $(GO_MODULES)
	cargo fmt --all

format-check:
	bunx prettier --check .

# Regenerates protocol types from the JSON Schemas and resolves fixtures (output is checked in).
codegen:
	bun run protocol/codegen/main.ts
	bun run daemon/scripts/fixtures.ts

# Fails if the checked-in generated types are stale.
codegen-check:
	bun run protocol/codegen/main.ts --check
	bun run daemon/scripts/fixtures.ts --check
