# protocol

Source of truth for everything on the bus. `schemas/*.schema.json` are JSON Schema 2020-12; `task codegen` generates:

- `gen/types.ts` (TS, import as `@neoworks-dev/neo-protocol/types`)
- `go/types.gen.go` (Go, package `neoworks.dev/suite/protocol/go`)
- `rust/src/types.rs` (Rust, crate `suite-protocol`)
- `examples/*.json` from the typed `examples/examples.ts`

Generated files are checked in; `task check` fails if they are stale. Runtime validation: `@neoworks-dev/neo-protocol/validate` (ajv).

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
- Any node may carry `"if": "path"` (dropped when the value is falsy or an empty array) and `"each": "path"` with optional `"as": "name"` (default `item`) and `"limit": n`: the node repeats once per element (at most n times), spliced into its parent's children. Inside, `{{index}}` is the element's 1-based position as text, e.g. `"label": "Download {{index}}", "key": "{{index}}"`.

The resolved tree must validate against `Component`; otherwise the result falls back to a generic card (text output, diffs, or the data as key–value lines). Examples: `fixtures/*/fixture.json`.

A template may be a neoui program instead: `"templates": {"<tool>": {"neoui": "<markup>"}}`. The daemon gives the result's `structuredContent` to the program as the value `data`, evaluates it with the `suite@1` library and sends an `openui` component (`{type: "openui", ui: UiDoc}`); a program with no `root` falls back to the generic card too. The format, the library and the actions are in [docs/neoui.md](../docs/neoui.md).

## Components

Apps add their own neoui components, built from the library, for the agent to use, in `app.presence.components` (keyed by name):

```jsonc
"components": {
  "BrightnessCard": {
    "description": "An image with a brightness slider and live preview. Use it to show a photo the user may adjust.",
    "params": { "asset": "string", "name": "string?" },
    "neoui": "<Card title={name}>\n  <Query id=\"preview\" hx-get=\"photo-editor.preview\" hx-vals={{ asset: asset, brightness: $level }} default={{ asset: asset }}/>\n  <Image src={preview.asset} alt={name}/>\n  <Slider name=\"level\" min={-100} max={100} step={5} value={0}/>\n  <Button variant=\"primary\" hx-post=\"photo-editor.brightness\" hx-vals={{ asset: asset, amount: $level }}>Apply</Button>\n</Card>"
  }
}
```

- **Name.** A PascalCase identifier (`[A-Z][A-Za-z0-9]*`). The agent always writes a component with its app: the app id, a dot and the name, `<photo-editor.BrightnessCard asset="…"/>` (an app id is `^[a-z][a-z0-9-]{0,31}$`). There are no plain names and no clashes: `photo-editor.BrightnessCard`, `notes.BrightnessCard` and `notes.Card` all exist next to each other and next to the library's `Card`, whatever the order the apps registered in. A plain `<BrightnessCard>` is an unknown component whose error says `did you mean photo-editor.BrightnessCard?`. `<app connected … components=…/>` and `ui_library` give each component as it is written. A registration is never refused for its components: one that is not usable (a name that isn't PascalCase, an empty description, untyped params, a body that is not neoui or does not draw) is left out and logged, and the app connects with the others.
- **`params`** are the attributes, by name. Each type is `string`, `number`, `boolean` or `any`, optionally an array (`string[]`) and optionally optional (`string?`); optional ones come last. They are data: a component cannot take components.
- **`neoui`** is a body in neoui over the `suite@1` library (not over other app components) with the use's attributes in scope by name (`asset`) and as `args` (`args.asset`). Fields, `Query`, requests (`hx-post`) and forms work as they do in the agent's own program. It is checked at registration: it has to parse without repairs and draw (with sample arguments). A component registered with an `openui` body is left out.
- **Each use is a program of its own**: two `photo-editor.BrightnessCard`s in one reply do not share `level`, queries or keys. Renderers only ever see the library components it draws; the keys of its fields and buttons are `<where it sits>/<key in the component>`.
- **Kept** by the daemon like templates, so docs that use a component still draw after the app closes. The agent hears of the components of connected apps: `components=` in the `<app connected …/>` line (a change of them is a new version, `<app updated …/>`), and `ui_library` returns their signatures and descriptions ([docs/neoui.md](../docs/neoui.md#how-the-agent-learns-it)).
