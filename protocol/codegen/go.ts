// Go emitter. Unions become a wrapper struct `U{Value UVariant}` whose UnmarshalJSON switches on the
// tag; variant structs omit the tag field and add it back in MarshalJSON. Unknown tags decode into
// `UnknownU` and re-encode unchanged, so older clients survive new event types.
import type { Def, Model, TypeExpr } from "./model.ts";

const INITIALISMS = new Set(["id", "url", "uri", "pid", "api", "json", "http", "ui"]);

export function goName(s: string): string {
  return s
    .split(/[^A-Za-z0-9]+|(?=[A-Z])/)
    .filter(Boolean)
    .map((w) => {
      const lower = w.toLowerCase();
      if (INITIALISMS.has(lower)) return lower.toUpperCase();
      return lower.charAt(0).toUpperCase() + lower.slice(1);
    })
    .join("");
}

function comment(text: string | undefined, indent = ""): string {
  if (!text) return "";
  return `${text
    .split("\n")
    .map((l) => `${indent}// ${l}`)
    .join("\n")}\n`;
}

export function generateGo(model: Model): string {
  const goType = (t: TypeExpr): string => {
    switch (t.kind) {
      case "string":
        return "string";
      case "integer":
        return "int64";
      case "number":
        return "float64";
      case "boolean":
        return "bool";
      case "any":
        return "json.RawMessage";
      case "ref":
        return t.name;
      case "array":
        return `[]${goType(t.items)}`;
      case "map":
        return `map[string]${goType(t.values)}`;
    }
  };
  // Optional scalars, structs, enums and unions become pointers so absent ≠ zero value.
  // Slices, maps and raw JSON are already nil-able.
  const optionalType = (t: TypeExpr): string => {
    if (t.kind === "array" || t.kind === "map" || t.kind === "any") return goType(t);
    return `*${goType(t)}`;
  };

  let body = "";
  const emitObject = (def: Extract<Def, { kind: "object" }>): void => {
    body += comment(def.description ? `${def.name}: ${def.description}` : undefined);
    body += `type ${def.name} struct {\n`;
    for (const f of def.fields) {
      body += comment(f.description, "\t");
      const type = f.required ? goType(f.type) : optionalType(f.type);
      const tag = f.required ? f.name : `${f.name},omitempty`;
      body += `\t${goName(f.name)} ${type} \`json:"${tag}"\`\n`;
    }
    body += "}\n\n";
    if (def.tag) {
      const { union, field, value } = def.tag;
      body += `func (${def.name}) is${union}() {}\n\n`;
      body += `// MarshalJSON adds the "${field}" tag.\n`;
      body += `func (v ${def.name}) MarshalJSON() ([]byte, error) {\n`;
      body += `\ttype plain ${def.name}\n`;
      body += `\treturn json.Marshal(struct {\n\t\tTag string \`json:"${field}"\`\n\t\tplain\n\t}{${JSON.stringify(value)}, plain(v)})\n}\n\n`;
    }
  };

  for (const def of model.defs) {
    switch (def.kind) {
      case "enum":
        body += comment(def.description ? `${def.name}: ${def.description}` : undefined);
        body += `type ${def.name} string\n\nconst (\n`;
        for (const v of def.values)
          body += `\t${def.name}${goName(v)} ${def.name} = ${JSON.stringify(v)}\n`;
        body += ")\n\n";
        break;
      case "object":
        emitObject(def);
        break;
      case "union": {
        const n = def.name;
        body += comment(
          `${def.description ? `${n}: ${def.description}` : n}\nValue is one of: ${def.variants.join(", ")}, Unknown${n}.`,
        );
        body += `type ${n} struct {\n\tValue ${n}Variant\n}\n\n`;
        body += `// ${n}Variant is implemented by every ${n} variant.\n`;
        body += `type ${n}Variant interface {\n\tis${n}()\n}\n\n`;
        body += `// Unknown${n} holds a variant this version doesn't know. It re-encodes unchanged.\n`;
        body += `type Unknown${n} struct {\n\tTag string\n\tRaw json.RawMessage\n}\n\n`;
        body += `func (Unknown${n}) is${n}() {}\n\n`;
        body += `// MarshalJSON returns the original JSON.\n`;
        body += `func (v Unknown${n}) MarshalJSON() ([]byte, error) {\n\treturn v.Raw, nil\n}\n\n`;
        body += `// MarshalJSON encodes the variant, including its tag.\n`;
        body += `func (u ${n}) MarshalJSON() ([]byte, error) {\n`;
        body += `\tif u.Value == nil {\n\t\treturn nil, errors.New("protocol: ${n} has no value")\n\t}\n`;
        body += `\treturn json.Marshal(u.Value)\n}\n\n`;
        body += `// UnmarshalJSON picks the variant from the "${def.tagField}" field.\n`;
        body += `func (u *${n}) UnmarshalJSON(data []byte) error {\n`;
        body += `\tvar head struct {\n\t\tTag *string \`json:"${def.tagField}"\`\n\t}\n`;
        body += `\tif err := json.Unmarshal(data, &head); err != nil {\n\t\treturn err\n\t}\n`;
        body += `\tif head.Tag == nil {\n\t\treturn errors.New("protocol: ${n} is missing \\"${def.tagField}\\"")\n\t}\n`;
        body += `\tswitch *head.Tag {\n`;
        for (const vname of def.variants) {
          const variant = model.byName.get(vname);
          if (variant?.kind !== "object" || !variant.tag) throw new Error(`bad variant ${vname}`);
          body += `\tcase ${JSON.stringify(variant.tag.value)}:\n`;
          body += `\t\tvar v ${vname}\n\t\tif err := json.Unmarshal(data, &v); err != nil {\n\t\t\treturn err\n\t\t}\n\t\tu.Value = v\n`;
        }
        body += `\tdefault:\n\t\tu.Value = Unknown${n}{Tag: *head.Tag, Raw: append(json.RawMessage(nil), data...)}\n\t}\n\treturn nil\n}\n\n`;
        break;
      }
    }
  }

  const imports = ['"encoding/json"'];
  if (model.defs.some((d) => d.kind === "union")) imports.push('"errors"');
  return `// Code generated by protocol/codegen from protocol/schemas. DO NOT EDIT.\n\npackage protocol\n\nimport (\n${imports.map((i) => `\t${i}`).join("\n")}\n)\n\n${body}`;
}
