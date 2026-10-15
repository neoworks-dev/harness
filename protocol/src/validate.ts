// Runtime validation of bus frames against the JSON Schemas (the generated types are compile-time only).
import Ajv2020, { type ErrorObject } from "ajv/dist/2020.js";
import envelopeSchema from "../schemas/envelope.schema.json";
import presenceSchema from "../schemas/presence.schema.json";
import sessionSchema from "../schemas/session.schema.json";
import viewSchema from "../schemas/view.schema.json";
import type { Envelope } from "../gen/types.ts";

const ajv = new Ajv2020({ discriminator: true, allErrors: false, strict: true });
ajv.addSchema([viewSchema, sessionSchema, presenceSchema, envelopeSchema]);

const envelopeValidator = ajv.getSchema("envelope.schema.json#/$defs/Envelope");
if (!envelopeValidator) throw new Error("protocol: Envelope schema missing");

export type ValidationResult = { ok: true; value: Envelope } | { ok: false; error: string };

function describe(errors: ErrorObject[] | null | undefined): string {
  return (errors ?? []).map((e) => `${e.instancePath || "/"} ${e.message ?? "invalid"}`).join("; ");
}

/** Checks a parsed frame against the Envelope schema. */
export function validateEnvelope(value: unknown): ValidationResult {
  if (envelopeValidator?.(value)) return { ok: true, value: value as Envelope };
  return { ok: false, error: describe(envelopeValidator?.errors) };
}

/** Validates any named def, e.g. `session.schema.json#/$defs/SessionEvent`. */
export function validateDef(ref: string, value: unknown): string | undefined {
  const validator = ajv.getSchema(ref);
  if (!validator) throw new Error(`protocol: unknown schema ${ref}`);
  return validator(value) ? undefined : describe(validator.errors);
}
