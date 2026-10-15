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

## Running it

```sh
make build                 # bin/suite
bun run daemon             # starts the daemon in the foreground (Ctrl-C stops it)
bin/suite attach --new     # new session in the current directory; type to chat
bin/suite status           # daemon info + session counts
bin/suite sessions         # sessions by last activity
bin/suite attach <id>      # id or unique prefix; replays history, then follows live
```

In `attach`: Ctrl-C cancels the running turn (exits when idle), Ctrl-D exits. Piped stdin sends one input per line and exits once the session is idle.

| Env                                   | Default                                                     |
| ------------------------------------- | ----------------------------------------------------------- |
| `SUITE_SOCKET`                        | `$XDG_RUNTIME_DIR/suite/bus.sock` (else `/tmp/suite-$UID/`) |
| `SUITE_DATA_DIR`                      | `$XDG_DATA_HOME/suite` (else `~/.local/share/suite`)        |
| `SUITE_MODEL`                         | `deepseek/deepseek-v4-flash` (`provider/model-id`)          |
| `SUITE_THINKING`                      | `off` (`minimal`…`xhigh`)                                   |
| `<PROVIDER>_API_KEY`, `SUITE_API_KEY` | falls back to API-key entries in `~/.pi/agent/auth.json`    |
| `SUITE_AGENT`                         | `pi`; `echo` is an offline scripted agent for tests         |

`model` and `thinkingLevel` can also be set in `~/.config/suite/config.json`; env wins.

## Commands

```sh
make check      # install, codegen check, format check, lint, typecheck, test, build — everything
make codegen    # regenerate protocol types after editing protocol/schemas
make build      # builds bin/suite
```
