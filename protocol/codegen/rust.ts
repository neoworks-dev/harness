// Rust emitter (serde). Unions are enums with one newtype variant per schema variant plus
// Unknown(serde_json::Value); (de)serialization is hand-written so the tag goes in and out of
// the variant's JSON object and unknown tags survive a round trip, like the Go types.
import type { Model, TypeExpr } from "./model.ts";

const KEYWORDS = new Set(["type", "ref", "move", "match", "self", "use", "mod", "fn", "loop"]);

function snake(name: string): string {
  const s = name.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
  return KEYWORDS.has(s) ? `r#${s}` : s;
}

function pascal(value: string): string {
  return value
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join("");
}

function doc(text: string | undefined, indent = ""): string {
  if (!text) return "";
  return `${text
    .split("\n")
    .map((l) => `${indent}/// ${l}`)
    .join("\n")}\n`;
}

function rustType(t: TypeExpr): string {
  switch (t.kind) {
    case "string":
      return "String";
    case "integer":
      return "i64";
    case "number":
      return "f64";
    case "boolean":
      return "bool";
    case "any":
      return "serde_json::Value";
    case "ref":
      return t.name;
    case "array":
      return `Vec<${rustType(t.items)}>`;
    case "map":
      return `BTreeMap<String, ${rustType(t.values)}>`;
  }
}

export function generateRust(model: Model): string {
  let body = "";
  for (const def of model.defs) {
    switch (def.kind) {
      case "enum": {
        body += doc(def.description);
        body += "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]\n";
        body += `pub enum ${def.name} {\n`;
        for (const v of def.values)
          body += `    #[serde(rename = ${JSON.stringify(v)})]\n    ${pascal(v)},\n`;
        body += "}\n\n";
        break;
      }
      case "object": {
        body += doc(def.description);
        if (def.tag)
          body += doc(
            `Tagged \`${def.tag.field}: "${def.tag.value}"\` inside [\`${def.tag.union}\`].`,
          );
        body += "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n";
        body += `pub struct ${def.name} {\n`;
        for (const f of def.fields) {
          body += doc(f.description, "    ");
          const type = rustType(f.type);
          if (f.required) {
            body += `    #[serde(rename = ${JSON.stringify(f.name)})]\n`;
            body += `    pub ${snake(f.name)}: ${type},\n`;
          } else {
            body += `    #[serde(rename = ${JSON.stringify(f.name)}, default, skip_serializing_if = "Option::is_none")]\n`;
            body += `    pub ${snake(f.name)}: Option<${type}>,\n`;
          }
        }
        body += "}\n\n";
        break;
      }
      case "union": {
        const n = def.name;
        body += doc(def.description);
        body += "#[derive(Debug, Clone, PartialEq)]\n";
        body += `pub enum ${n} {\n`;
        const variants: { name: string; tag: string }[] = [];
        for (const vname of def.variants) {
          const variant = model.byName.get(vname);
          if (variant?.kind !== "object" || !variant.tag) throw new Error(`bad variant ${vname}`);
          variants.push({ name: vname, tag: variant.tag.value });
          body += `    ${vname}(${vname}),\n`;
        }
        body += "    /// A variant this version doesn't know; re-serializes unchanged.\n";
        body += "    Unknown(serde_json::Value),\n}\n\n";
        const tag = JSON.stringify(def.tagField);
        body += `impl Serialize for ${n} {\n`;
        body +=
          "    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {\n";
        body += "        let (tag, value) = match self {\n";
        for (const v of variants) {
          body += `            ${n}::${v.name}(v) => (${JSON.stringify(v.tag)}, serde_json::to_value(v).map_err(S::Error::custom)?),\n`;
        }
        body += `            ${n}::Unknown(v) => return v.serialize(serializer),\n        };\n`;
        body += `        with_tag(${tag}, tag, value).serialize(serializer)\n    }\n}\n\n`;
        body += `impl<'de> Deserialize<'de> for ${n} {\n`;
        body +=
          "    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {\n";
        body += "        let value = serde_json::Value::deserialize(deserializer)?;\n";
        body += `        let tag = value.get(${tag}).and_then(|t| t.as_str()).ok_or_else(|| D::Error::missing_field(${tag}))?;\n`;
        body += "        match tag {\n";
        for (const v of variants) {
          body += `            ${JSON.stringify(v.tag)} => serde_json::from_value(value).map(${n}::${v.name}).map_err(D::Error::custom),\n`;
        }
        body += `            _ => Ok(${n}::Unknown(value)),\n        }\n    }\n}\n\n`;
        for (const v of variants) {
          body += `impl From<${v.name}> for ${n} {\n    fn from(v: ${v.name}) -> Self {\n        ${n}::${v.name}(v)\n    }\n}\n\n`;
        }
        break;
      }
    }
  }
  return `// Code generated by protocol/codegen from protocol/schemas. DO NOT EDIT.
#![allow(clippy::large_enum_variant)]

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

/// Puts the union tag into a variant's JSON object.
fn with_tag(field: &str, tag: &str, value: serde_json::Value) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(field.to_string(), serde_json::Value::String(tag.to_string()));
    if let serde_json::Value::Object(fields) = value {
        map.extend(fields);
    }
    serde_json::Value::Object(map)
}

${body}`;
}
