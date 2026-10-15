# protocol

Source of truth for everything on the bus. `schemas/*.schema.json` are JSON Schema 2020-12; `make codegen` generates:

- `gen/types.ts` (TS, import as `@suite/protocol/types`)
- `go/types.gen.go` (Go, package `neoworks.dev/suite/protocol/go`)
- `rust/src/types.rs` (Rust, crate `suite-protocol`)
- `examples/*.json` from the typed `examples/examples.ts`

Generated files are checked in; `make check` fails if they are stale. Runtime validation: `@suite/protocol/validate` (ajv).

## Why a custom generator

Off-the-shelf Go generators flatten `oneOf` tagged unions into one struct of optional fields. `codegen/` is small and only accepts a restricted subset, which gives real tagged unions in all three languages. In Rust a union is an enum with one newtype variant per schema variant plus `Unknown(serde_json::Value)`; its serde impls are generated so unknown tags round-trip, as in Go.

## Schema conventions

- Every named type lives in `$defs` with a PascalCase name, unique across all files. Cross-file refs: `session.schema.json#/$defs/SessionInfo`.
- Objects use `additionalProperties: false` and camelCase properties. No inline objects or enums; give them a name.
- Maps: `{"type": "object", "additionalProperties": <schema>}`.
- Tagged unions: `{"type": "object", "oneOf": [$refs…], "discriminator": {"propertyName": "type"}}`. Each variant declares the tag as a required string `const` and belongs to exactly one union. The bus envelope is tagged by `topic`; everything else by `type`.
- Optional means "not in `required`" — never `null`. Optional arrays/maps: absent and empty are equivalent.
- `{}` (only a description) is opaque JSON: `unknown` in TS, `json.RawMessage` in Go.
- Readers must tolerate unknown union tags: Go decodes them into `Unknown<Union>`, Rust into `Union::Unknown(value)` (both re-encode unchanged); TS consumers should ignore them. String enums are strict in Rust: a new enum value needs regenerated types.

## Go usage

```go
var env protocol.Envelope
json.Unmarshal(frame, &env)
switch m := env.Value.(type) {
case protocol.SessionEvents:
    switch ev := m.Payload.Event.Value.(type) {
    case protocol.AgentMessageChunk:
        fmt.Print(ev.Text)
    }
}
json.Marshal(protocol.Envelope{Value: protocol.SessionList{ID: "1"}})
```

## Templates

Apps register view templates for their tool results in `app.presence.templates` (keyed by tool name, without the app prefix); the result's `structuredContent` is the template's `data`. A template is a component tree in the view format (`view.schema.json`) with three differences:

- `text` takes `markdown` (a string) instead of `blocks`; `diff` takes `oldText`/`newText` instead of `lines`. The daemon parses and diffs them.
- Any string may contain `{{path}}` (dotted: `data.size.w`, `item.name`, `data.rows.0`); missing values become empty. A string that is exactly `{{path}}` takes the raw value, so `"rows": "{{data.rows}}"` passes an array and `"value": "{{data.progress}}"` a number. `kv.items` may be an object (key → value); `table.rows` may be objects, read in column order.
- Any node may carry `"if": "path"` (dropped when the value is falsy or an empty array) and `"each": "path"` with optional `"as": "name"` (default `item`): the node repeats once per element, spliced into its parent's children.

The resolved tree must validate against `Component`; otherwise the result falls back to a generic card (text output, diffs, or the data as key–value lines). Examples: `fixtures/*/fixture.json`.
