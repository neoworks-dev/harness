# lsp service

Language servers as a background connector (`kind: service`), so the agent gets diagnostics and navigation whether or not an editor is open. The daemon starts it (`SUITE_SERVICES=none` to skip; it exits when the daemon does); by hand: `bun run services/lsp/main.ts`.

| Tool          | Args                                 | Does                                            |
| ------------- | ------------------------------------ | ----------------------------------------------- |
| `diagnostics` | `path`                               | Errors and warnings (waits for fresh results)   |
| `definition`  | `path`, `line`, `symbol` or `column` | Where the symbol is defined                     |
| `references`  | same                                 | Every reference, with the line's text           |
| `hover`       | same                                 | Type and docs                                   |
| `rename`      | same + `newName`                     | Renames across the project and writes the files |

Paths are absolute, lines 1-based; `symbol` (the identifier on that line) saves the model from counting columns.

Servers start on first use, one per language and project root (`go.work`/`go.mod`, `tsconfig.json`/`package.json`, `Cargo.toml`), and keep running: `gopls`, `typescript-language-server --stdio`, `rust-analyzer` — whichever are on `PATH`. Files are re-read from disk before every request, so edits from anywhere are seen without bookkeeping.

Installed plugins add servers (`lspServers` in the marketplace entry or the manifest, or `.lsp.json`): `{ command, args, extensionToLanguage }`, project root by `.git`. They come after the three above, are read on every file (a plugin installed now is used at once), and need their program on `PATH`.

Rough edges: no workspace-wide diagnostics (one file at a time), no code actions or formatting, the servers' settings are their defaults.
