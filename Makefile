# One entry point for both toolchains. `make check` builds, lints and tests everything.
.NOTPARALLEL:

GO_MODULES := cli sdk-go renderers/tui protocol/go apps/photo
# Named, so rustfmt leaves path dependencies outside this repo (svelte-gpui's host) alone.
RUST_FMT := cargo fmt -p suite-protocol -p suite-sdk -p suite-renderer-gpui -p suite-renderer-ratatui -p suite-pad -p suite-term -p suite-gui

.PHONY: check install build test test-ts test-go test-rust lint lint-ts lint-go lint-rust typecheck format format-check codegen codegen-check

check: install codegen-check format-check lint typecheck build test

install:
	bun install --frozen-lockfile

build: bin/suite bin/photo bin/suite-term bin/suite-gui

bin/suite: FORCE
	go build -o bin/suite ./cli

# The photo editor stand-in app (apps/photo).
bin/photo: FORCE
	go build -o bin/photo ./apps/photo

# The terminal app (apps/term): what `suite` with no arguments opens.
bin/suite-term: FORCE
	cargo build --release -p suite-term
	cp target/release/suite-term bin/suite-term.tmp && mv bin/suite-term.tmp bin/suite-term

# The desktop app (apps/gui): its Svelte UI as a bundle, run by the binary (`suite gui`).
# Needs svelte-gpui checked out next to this repo.
bin/suite-gui: FORCE
	bun apps/gui/build.mjs
	cargo build --release -p suite-gui
	cp target/release/suite-gui bin/suite-gui.tmp && mv bin/suite-gui.tmp bin/suite-gui

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
	$(RUST_FMT) --check
	cargo clippy --workspace --all-targets --quiet -- -D warnings

typecheck:
	bunx tsc -p tsconfig.json --noEmit
	cd apps/gui && bunx svelte-check --tsconfig ./tsconfig.json --fail-on-warnings

format:
	bunx prettier --write .
	gofmt -w $(GO_MODULES)
	$(RUST_FMT)

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
