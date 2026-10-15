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
