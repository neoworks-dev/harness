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
- Any node may carry `"if": "path"` (dropped when the value is falsy or an empty array) and `"each": "path"` with optional `"as": "name"` (default `item`) and `"limit": n`: the node repeats once per element (at most n times), spliced into its parent's children. Inside, `{{index}}` is the element's 1-based position as text, e.g. `"label": "Download {{index}}", "key": "{{index}}"`.

The resolved tree must validate against `Component`; otherwise the result falls back to a generic card (text output, diffs, or the data as key–value lines). Examples: `fixtures/*/fixture.json`.

A template may be a program in OpenUI Lang instead: `"templates": {"<tool>": {"openui": "<program>"}}`. The daemon appends the result's `structuredContent` as a last statement `data = <value>`, evaluates the program with the `suite@1` library and sends an `openui` component (`{type: "openui", ui: UiDoc}`); a program with no `root` falls back to the generic card too. The format, the library and the actions are in [docs/openui.md](../docs/openui.md).

## Components

Apps add their own OpenUI components, built from the library, for the agent to use, in `app.presence.components` (keyed by name):

```jsonc
"components": {
  "BrightnessCard": {
    "description": "An image with a brightness slider and live preview. Use it to show a photo the user may adjust.",
    "params": { "asset": "string", "name": "string?" },
    "openui": "$level = 0\nroot = Card([pic, slider, apply], args.name)\npic = Image(preview.asset, args.name)\nslider = Slider(\"level\", -100, 100, 5, $level)\npreview = Query(\"photo-editor.preview\", { asset: args.asset, brightness: $level }, { asset: args.asset })\nsave = Mutation(\"photo-editor.brightness\", { asset: args.asset, amount: $level })\napply = Button(\"Apply\", @Run(save), \"primary\")"
  }
}
```

- **Name.** A PascalCase identifier (`[A-Z][A-Za-z0-9]*`): lang-core reads `photo-editor.X` as a subtraction and a member access, so the app is not part of the name. One that the library, the language (`Query`, `Mutation`, `Action`) or another app already has rejects the whole registration (`bad_request` on the presence, and the app does not connect), so pick a name that says which app it is for.
- **`params`** are the arguments, positional in this order (props are only ever added at the end, as in the library). Each type is `string`, `number`, `boolean` or `any`, optionally an array (`string[]`) and optionally optional (`string?`); optional ones come last. They are data: a component cannot take components.
- **`openui`** is a program over the `suite@1` library (not over other app components) with the call's arguments as the object `args` (`args.asset`). The daemon appends `args = {…}` as its last statement and evaluates it like any program, so `$state`, `Query`, `Mutation`, `@Run` and forms work as they do in the agent's own. It is checked at registration: it has to parse and have a `root`.
- **Each use is a program of its own**: two `BrightnessCard`s in one reply do not share `$level`, queries or keys. Renderers only ever see the library components it draws; the keys of its fields and buttons are `<where it sits>/<key in the component>`.
- **Kept** by the daemon like templates, so docs that use a component still draw after the app closes.
