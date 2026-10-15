# suite

One agent session shared across every app in the suite. The design is in [docs/design.md](docs/design.md).

| Path                 | What                                                       | Phase |
| -------------------- | ---------------------------------------------------------- | ----- |
| `protocol/`          | JSON Schemas (source of truth) + generated TS and Go types | 0     |
| `daemon/`            | TS on Bun: session store, bus hub, turn queue              | 1     |
| `harness/`           | TS on Bun: pi-based agent, core tools, system prompt       | 1     |
| `cli/`               | Go: `suite status / sessions / attach`                     | 1     |
| `sdk-ts/`, `sdk-go/` | Bus clients                                                | 2     |
| `sdk-rs/`            | Rust bus client for GPUI apps                              | 3     |
| `renderers/`         | web (TS), gpui (Rust), tui (Go)                            | 4     |
| `fixtures/`          | View fixtures + renderer snapshot tests                    | 4     |

## Requirements

Bun ≥ 1.3, Go ≥ 1.27.

## Commands

```sh
make check      # install, codegen check, format check, lint, typecheck, test, build — everything
make codegen    # regenerate protocol types after editing protocol/schemas
make build      # builds bin/suite
```
