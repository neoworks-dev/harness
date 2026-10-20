// One OpenUI program, running: parsed and evaluated with lang-core into the UiDoc renderers draw,
// with the state a user changes (fields, $variables, queries) kept beside it. Nothing here knows
// about sessions or the wire; the daemon owns that (docs/openui.md).
import {
  ACTION_STEPS,
  autoClose,
  createQueryManager,
  createStore,
  createStreamingParser,
  evaluate,
  evaluateRoot,
  split,
  tokenize,
  validate,
  type ActionStep,
  type ASTNode,
  type ElementNode,
  type EvaluationContext,
  type OpenUIError,
  type ParseResult,
  type QueryManager,
  type Store,
} from "@openuidev/lang-core";
import type { Block, UiDoc, UiError } from "@suite/protocol/types";
import { Converter, isRecord, type ButtonInfo, type Field } from "./convert.ts";
import { library } from "./library.ts";

const schema = library.toJSONSchema();

/** Runs the tool a Query or a Mutation names; a mutation is something the user did. */
export type CallTool = (
  name: string,
  args: Record<string, unknown>,
  kind: "query" | "mutation",
) => Promise<unknown>;

/** What a program needs from its host. */
export interface ProgramEnv {
  /** Markdown to blocks; Text and Callout carry both their text and the blocks. */
  markdown: (source: string) => Block[];
  /** Without it queries keep their defaults and mutations fail. */
  callTool?: CallTool;
}

/** What a pressed button asks of the host: a message for the agent. */
export interface AssistantMessage {
  message: string;
  /** The button that was pressed, and the form it is in. */
  label: string;
  form?: string;
  /** The `@ToAssistant` context, any JSON. */
  context?: unknown;
  /** The values of the button's form, or the program's $variables outside one. */
  values?: Record<string, unknown>;
}

export interface PressResult {
  /**
   * Why nothing (or not everything) ran: no such button, the doc is still streaming, a field of
   * its form failed its rules, or a mutation failed and stopped the rest.
   */
  refused?: "unknown" | "streaming" | "invalid" | "halted";
  assistant: AssistantMessage[];
}

interface Rendered {
  doc: UiDoc;
  fields: Map<string, Field>;
  buttons: Map<string, ButtonInfo>;
}

/** The longest text a field takes or a message carries. */
const TEXT_LIMIT = 100_000;

/** The id of the statement an unfinished program ends in, when it ends unfinished. */
function pendingStatement(source: string): string | undefined {
  const closed = autoClose(source.trim());
  if (!closed.wasIncomplete) return undefined;
  return split(tokenize(closed.text)).at(-1)?.id;
}

function record(value: unknown): Record<string, unknown> {
  if (isRecord(value)) return value;
  return {};
}

function uiError(error: { code: string; message: string; statementId?: string }): UiError {
  const out: UiError = { code: error.code, message: error.message };
  if (error.statementId !== undefined) out.statementId = error.statementId;
  return out;
}

export class Program {
  private readonly store: Store = createStore();
  private readonly queries: QueryManager;
  private readonly parser = createStreamingParser(schema, library.root);
  private readonly values = new Map<string, unknown>();
  private readonly errors = new Map<string, string>();
  private readonly context: EvaluationContext;
  private source = "";
  private streaming = true;
  private parsed: ParseResult | undefined;
  private crash: string | undefined;
  private pending: string | undefined;
  private rendered: Rendered | undefined;
  private rendering = false;
  private scheduled = false;
  private disposed = false;
  private mutating = false;
  /** The queries and mutations last handed to lang-core, as JSON. */
  private asked: string | undefined;
  /** Called when the doc changes without the host asking: a query answered, a mutation is running. */
  onChange: (() => void) | undefined;

  constructor(
    private readonly id: string,
    private readonly env: ProgramEnv,
  ) {
    const { callTool } = env;
    this.queries = createQueryManager(
      callTool
        ? {
            callTool: (name, args) => callTool(name, args, this.mutating ? "mutation" : "query"),
          }
        : null,
    );
    this.queries.activate();
    this.queries.subscribe(() => this.later());
    this.context = {
      getState: (name) => this.store.get(name),
      resolveRef: (name) => this.queries.getMutationResult(name) ?? this.queries.getResult(name),
      actions: schema.actions,
    };
  }

  /** Sets the program text. `streaming` is true while more of it may still arrive. */
  set(source: string, streaming: boolean): void {
    if (source === this.source && streaming === this.streaming && this.parsed) return;
    this.source = source;
    this.streaming = streaming;
    this.crash = undefined;
    try {
      this.parsed = this.parser.set(source, !streaming);
    } catch (err) {
      this.parsed = undefined;
      this.crash = err instanceof Error ? err.message : String(err);
    }
    this.pending = streaming ? pendingStatement(source) : undefined;
    this.rendered = undefined;
  }

  /** The evaluated doc. */
  doc(): UiDoc {
    return this.current().doc;
  }

  dispose(): void {
    this.disposed = true;
    this.queries.dispose();
  }

  /** A field's new value. False when there is no such field, the value isn't one it takes, or the doc is streaming. */
  input(key: string, value: unknown): boolean {
    const field = this.current().fields.get(key);
    if (!field || this.streaming) return false;
    const next = this.coerce(field, value);
    if (next === undefined) return false;
    if (field.bound !== undefined) this.store.set(field.bound, next);
    else this.values.set(field.id, next);
    // A field that failed once is checked again as it changes, so its message goes when it's fixed.
    if (this.errors.has(field.id)) this.check(field);
    this.rendered = undefined;
    return true;
  }

  /** Presses a button: checks its form, runs its action plan, and returns what to tell the agent. */
  async press(key: string): Promise<PressResult> {
    const button = this.current().buttons.get(key);
    if (!button) return { refused: "unknown", assistant: [] };
    if (this.streaming) return { refused: "streaming", assistant: [] };
    // A form's button checks the fields first, unless all it does is move state or open a link.
    const submits = button.plan.steps.some(
      (step) =>
        step.type === ACTION_STEPS.ToAssistant ||
        step.type === ACTION_STEPS.Run ||
        step.type === "custom_action",
    );
    if (button.form !== undefined && submits && !this.validateForm(button.form)) {
      this.rendered = undefined;
      return { refused: "invalid", assistant: [] };
    }
    const assistant: AssistantMessage[] = [];
    try {
      for (const step of button.plan.steps) {
        if (!(await this.run(step, button, assistant))) return { refused: "halted", assistant };
      }
      return { assistant };
    } finally {
      this.rendered = undefined;
    }
  }

  // ─── Running actions ──────────────────────────────────────────────────────

  /** Runs one step; false halts the plan (a mutation that failed). */
  private async run(
    step: ActionStep,
    button: ButtonInfo,
    out: AssistantMessage[],
  ): Promise<boolean> {
    switch (step.type) {
      case ACTION_STEPS.Set:
        this.store.set(step.target, evaluate(step.valueAST, this.context));
        return true;
      case ACTION_STEPS.Reset: {
        const declared = this.parsed?.stateDeclarations ?? {};
        for (const target of step.targets) {
          this.store.set(target, declared[target] ?? null, { pristine: true });
        }
        return true;
      }
      case ACTION_STEPS.Run: {
        if (step.refType === "query") {
          this.queries.invalidate([step.statementId]);
          return true;
        }
        const mutation = this.parsed?.mutationStatements.find(
          (m) => m.statementId === step.statementId,
        );
        const args = mutation?.argsAST && evaluate(mutation.argsAST, this.context);
        // fireMutation calls the tool before it first yields, so the flag is set for exactly that call.
        this.mutating = true;
        const done = this.queries.fireMutation(step.statementId, record(args));
        this.mutating = false;
        return done;
      }
      case ACTION_STEPS.ToAssistant: {
        const message: AssistantMessage = {
          message: step.message.slice(0, TEXT_LIMIT),
          label: button.label,
        };
        if (button.form !== undefined) message.form = button.form;
        if (step.context !== undefined) message.context = step.context;
        const values = this.formValues(button.form);
        if (Object.keys(values).length > 0) message.values = values;
        out.push(message);
        return true;
      }
      default:
        // @OpenUrl and @Copy are for the client; the daemon has nothing to do for them.
        return true;
    }
  }

  private formValues(form: string | undefined): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    if (form === undefined) {
      for (const [name, value] of Object.entries(this.store.getSnapshot())) {
        if (name.startsWith("$") && value !== null && value !== undefined) out[name] = value;
      }
      return out;
    }
    for (const field of this.current().fields.values()) {
      if (field.form === form) out[field.name] = this.valueOf(field) ?? null;
    }
    return out;
  }

  // ─── Fields ───────────────────────────────────────────────────────────────

  private valueOf(field: Field): unknown {
    const raw =
      field.bound !== undefined
        ? this.store.get(field.bound)
        : (this.values.get(field.id) ?? field.literal);
    switch (field.component) {
      case "CheckBox":
      case "Switch":
        return raw === true;
      case "Slider": {
        const n = typeof raw === "string" && raw.trim() !== "" ? Number(raw) : raw;
        return typeof n === "number" && Number.isFinite(n) ? n : undefined;
      }
      default:
        if (typeof raw === "number" || typeof raw === "boolean") return String(raw);
        return typeof raw === "string" && raw !== "" ? raw : undefined;
    }
  }

  /** The value as the field keeps it; undefined when it isn't one this field takes. */
  private coerce(field: Field, value: unknown): unknown {
    switch (field.component) {
      case "CheckBox":
      case "Switch":
        return typeof value === "boolean" ? value : undefined;
      case "Slider":
        if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
        return Math.min(field.max ?? value, Math.max(field.min ?? value, value));
      case "Select":
      case "RadioGroup":
        if (typeof value !== "string") return undefined;
        return value === "" || field.options?.includes(value) === true ? value : undefined;
      default:
        return typeof value === "string" ? value.slice(0, TEXT_LIMIT) : undefined;
    }
  }

  /** The message a field's value earns, none when it passes. */
  private problem(field: Field): string | undefined {
    const value = this.valueOf(field);
    const toggle = field.component === "CheckBox" || field.component === "Switch";
    // A required box or switch is one the user has to turn on; off is not empty to lang-core.
    if (toggle && field.rules.some((rule) => rule.type === "required") && value !== true) {
      return "This field is required";
    }
    if (field.component === "DatePicker" && typeof value === "string") {
      if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return "Use the form YYYY-MM-DD";
    }
    return validate(toggle ? true : value, field.rules);
  }

  private check(field: Field): boolean {
    const message = this.problem(field);
    if (message === undefined) this.errors.delete(field.id);
    else this.errors.set(field.id, message);
    return message === undefined;
  }

  private validateForm(form: string): boolean {
    let ok = true;
    for (const field of this.current().fields.values()) {
      if (field.form === form && !this.check(field)) ok = false;
    }
    return ok;
  }

  // ─── Rendering ────────────────────────────────────────────────────────────

  /** A change that came on its own (a query answered): tell the host once, soon. */
  private later(): void {
    if (this.rendering || this.scheduled || this.disposed) return;
    this.scheduled = true;
    queueMicrotask(() => {
      this.scheduled = false;
      if (this.disposed) return;
      this.rendered = undefined;
      this.onChange?.();
    });
  }

  private current(): Rendered {
    if (!this.rendered) {
      this.rendering = true;
      try {
        this.rendered = this.build();
      } finally {
        this.rendering = false;
      }
    }
    return this.rendered;
  }

  private build(): Rendered {
    const doc: UiDoc = { id: this.id };
    const parsed = this.parsed;
    if (!parsed) {
      if (this.crash !== undefined && !this.streaming) {
        doc.errors = [{ code: "parse-exception", message: `Parser crashed: ${this.crash}` }];
      }
      return { doc, fields: new Map(), buttons: new Map() };
    }
    this.store.initialize(parsed.stateDeclarations, {});
    // Tools run for a finished program only; a streaming one is still being written.
    if (!this.streaming) this.startQueries(parsed);
    const evalErrors: OpenUIError[] = [];
    let root: ElementNode | null = null;
    try {
      root = evaluateRoot(parsed, {
        ctx: this.context,
        library,
        store: this.store,
        errors: evalErrors,
      });
    } catch (err) {
      const why = err instanceof Error ? err.message : String(err);
      evalErrors.push({
        source: "runtime",
        code: "runtime-error",
        message: `Evaluating the program failed: ${why}`,
      });
    }
    const converter = new Converter({
      isPending: (statement) => this.pending !== undefined && statement === this.pending,
      value: (field) => this.valueOf(field),
      error: (field) => this.errors.get(field.id),
      mutationLoading: (id) => this.queries.getMutationResult(id)?.status === "loading",
      markdown: (source) => this.env.markdown(source),
    });
    if (root) doc.root = converter.convert(root);
    if (this.streaming) {
      // What looks broken may only be unfinished: errors wait for the end.
      doc.streaming = true;
    } else {
      const errors = [
        ...parsed.meta.errors.filter((error) => error.severity !== "warning").map(uiError),
        ...evalErrors.map(uiError),
        ...this.queries.getSnapshot().__openui_errors.map(uiError),
      ];
      if (errors.length > 0) doc.errors = errors;
    }
    return { doc, fields: converter.fields, buttons: converter.buttons };
  }

  private toolName(ast: ASTNode | null): string {
    const name = ast && evaluate(ast, this.context);
    if (typeof name === "string") return name;
    return "";
  }

  /**
   * Hands the queries to lang-core only when what they ask changed. lang-core fetches a query
   * without data on every evaluateQueries, and a fetch's answer rebuilds the doc, so a failing
   * query asked on every build would be asked again forever.
   */
  private startQueries(parsed: ParseResult): void {
    const queries = parsed.queryStatements.map((query) => ({
      statementId: query.statementId,
      toolName: this.toolName(query.toolAST),
      args: query.argsAST ? evaluate(query.argsAST, this.context) : null,
      defaults: query.defaultsAST ? evaluate(query.defaultsAST, this.context) : null,
      deps: Object.fromEntries((query.deps ?? []).map((name) => [name, this.store.get(name)])),
      complete: query.complete,
    }));
    const mutations = parsed.mutationStatements.map((mutation) => ({
      statementId: mutation.statementId,
      toolName: this.toolName(mutation.toolAST),
    }));
    const asked = JSON.stringify([queries, mutations]);
    if (asked === this.asked) return;
    this.asked = asked;
    this.queries.evaluateQueries(queries);
    this.queries.registerMutations(mutations);
  }
}
