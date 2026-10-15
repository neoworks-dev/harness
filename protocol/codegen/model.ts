// Reads the schema files into a small language-neutral model that the emitters walk.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

export type TypeExpr =
  | { kind: "string" | "integer" | "number" | "boolean" | "any" }
  | { kind: "ref"; name: string }
  | { kind: "array"; items: TypeExpr }
  | { kind: "map"; values: TypeExpr };

export interface Field {
  name: string;
  type: TypeExpr;
  required: boolean;
  description?: string;
}

export type Def =
  | { kind: "object"; name: string; description?: string; fields: Field[]; tag?: VariantTag }
  | { kind: "enum"; name: string; description?: string; values: string[] }
  | { kind: "union"; name: string; description?: string; tagField: string; variants: string[] };

export interface VariantTag {
  union: string;
  field: string;
  value: string;
}

export interface Model {
  /** In file order (files sorted by name), so output is stable. */
  defs: Def[];
  byName: Map<string, Def>;
}

type Json = Record<string, unknown>;

function isObject(v: unknown): v is Json {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function propertiesOf(schema: Json): Json {
  if (isObject(schema.properties)) return schema.properties;
  return {};
}

function requiredOf(schema: Json): unknown[] {
  if (Array.isArray(schema.required)) return schema.required as unknown[];
  return [];
}

export function loadSchemas(dir: string): Model {
  const files = readdirSync(dir)
    .filter((f) => f.endsWith(".schema.json"))
    .sort();
  const raw = new Map<string, { file: string; schema: Json }>();
  for (const file of files) {
    const doc: unknown = JSON.parse(readFileSync(join(dir, file), "utf8"));
    if (!isObject(doc) || doc.$id !== file)
      throw new Error(`${file}: $id must equal the file name`);
    const defs = doc.$defs;
    if (!isObject(defs)) throw new Error(`${file}: missing $defs`);
    for (const [name, schema] of Object.entries(defs)) {
      if (!/^[A-Z][A-Za-z0-9]*$/.test(name))
        throw new Error(`${file}: def ${name} must be PascalCase`);
      if (raw.has(name)) throw new Error(`${file}: duplicate def name ${name}`);
      if (!isObject(schema)) throw new Error(`${file}#${name}: not an object`);
      raw.set(name, { file, schema });
    }
  }

  const resolveRef = (from: string, ref: string): string => {
    const m = /^([a-z.-]*\.schema\.json)?#\/\$defs\/([A-Za-z0-9]+)$/.exec(ref);
    if (!m) throw new Error(`${from}: unsupported $ref ${ref}`);
    const target = raw.get(m[2] ?? "");
    if (!target || target.file !== (m[1] ?? from))
      throw new Error(`${from}: unresolved $ref ${ref}`);
    return m[2] ?? "";
  };

  const typeExpr = (where: string, file: string, s: Json): TypeExpr => {
    if (typeof s.$ref === "string") return { kind: "ref", name: resolveRef(file, s.$ref) };
    if (s.enum !== undefined || s.oneOf !== undefined || s.properties !== undefined) {
      throw new Error(`${where}: inline enums/unions/objects are not supported; use a named def`);
    }
    switch (s.type) {
      case "string":
      case "integer":
      case "number":
      case "boolean":
        return { kind: s.type };
      case "array":
        if (!isObject(s.items)) throw new Error(`${where}: array needs items`);
        return { kind: "array", items: typeExpr(`${where}[]`, file, s.items) };
      case "object":
        if (!isObject(s.additionalProperties))
          throw new Error(`${where}: inline object; use a named def`);
        return { kind: "map", values: typeExpr(`${where}{}`, file, s.additionalProperties) };
      case undefined: {
        const keys = Object.keys(s).filter((k) => k !== "description");
        if (keys.length > 0) throw new Error(`${where}: untyped schema with ${keys.join(", ")}`);
        return { kind: "any" };
      }
      default:
        throw new Error(`${where}: unsupported type ${String(s.type)}`);
    }
  };

  const defs: Def[] = [];
  const byName = new Map<string, Def>();
  for (const [name, { file, schema }] of raw) {
    const where = `${file}#${name}`;
    const description = typeof schema.description === "string" ? schema.description : undefined;
    let def: Def;
    if (Array.isArray(schema.oneOf)) {
      const disc = schema.discriminator;
      if (!isObject(disc) || typeof disc.propertyName !== "string") {
        throw new Error(`${where}: oneOf needs discriminator.propertyName`);
      }
      const variants = schema.oneOf.map((v) => {
        if (!isObject(v) || typeof v.$ref !== "string")
          throw new Error(`${where}: oneOf entries must be $refs`);
        return resolveRef(file, v.$ref);
      });
      def = { kind: "union", name, description, tagField: disc.propertyName, variants };
    } else if (schema.type === "string" && Array.isArray(schema.enum)) {
      const values = schema.enum.map((v) => {
        if (typeof v !== "string") throw new Error(`${where}: enum values must be strings`);
        return v;
      });
      def = { kind: "enum", name, description, values };
    } else if (schema.type === "object" && schema.additionalProperties === false) {
      const props = propertiesOf(schema);
      const required = requiredOf(schema);
      const fields: Field[] = [];
      for (const [fname, fs] of Object.entries(props)) {
        if (!isObject(fs)) throw new Error(`${where}.${fname}: not an object`);
        if (fs.const !== undefined) continue; // union tags are handled below
        fields.push({
          name: fname,
          type: typeExpr(`${where}.${fname}`, file, fs),
          required: required.includes(fname),
          description: typeof fs.description === "string" ? fs.description : undefined,
        });
      }
      def = { kind: "object", name, description, fields };
    } else {
      throw new Error(
        `${where}: must be an object (additionalProperties: false), string enum or tagged oneOf`,
      );
    }
    defs.push(def);
    byName.set(name, def);
  }

  // Attach tags to variants and check each union.
  for (const def of defs) {
    if (def.kind !== "union") continue;
    const seen = new Set<string>();
    for (const vname of def.variants) {
      const variant = byName.get(vname);
      const schema = raw.get(vname)?.schema;
      const where = `${def.name} → ${vname}`;
      if (variant?.kind !== "object" || !schema)
        throw new Error(`${where}: variants must be object defs`);
      if (variant.tag) throw new Error(`${where}: variant already belongs to ${variant.tag.union}`);
      const tagSchema = propertiesOf(schema)[def.tagField];
      const required = requiredOf(schema);
      if (
        !isObject(tagSchema) ||
        typeof tagSchema.const !== "string" ||
        !required.includes(def.tagField)
      ) {
        throw new Error(`${where}: needs required "${def.tagField}" with a string const`);
      }
      if (seen.has(tagSchema.const)) throw new Error(`${where}: duplicate tag ${tagSchema.const}`);
      seen.add(tagSchema.const);
      variant.tag = { union: def.name, field: def.tagField, value: tagSchema.const };
    }
  }
  // Consts are only allowed as union tags.
  for (const [name, { schema }] of raw) {
    const props = propertiesOf(schema);
    for (const [fname, fs] of Object.entries(props)) {
      const def = byName.get(name);
      if (
        isObject(fs) &&
        fs.const !== undefined &&
        (def?.kind !== "object" || def.tag?.field !== fname)
      ) {
        throw new Error(`${name}.${fname}: const is only allowed as a union tag`);
      }
    }
  }
  return { defs, byName };
}
