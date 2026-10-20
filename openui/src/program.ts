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
  parseExpression,
  split,
  tokenize,
  validate,
  isReactiveAssign,
  type ActionStep,
  type ASTNode,
  type ElementNode,
  type EvaluationContext,
  type OpenUIError,
  type ParseResult,
  type QueryManager,
  type Store,
} from "@openuidev/lang-core";
import type { Block, UiDoc, UiError, UiNode } from "@suite/protocol/types";
import { type AppLibrary, instanceProgram, SUITE_ONLY } from "./components.ts";
import { Converter, isElement, isPlan, isRecord, type ButtonInfo, type Field } from "./convert.ts";

/** The actions are the suite's whatever apps add. */
const schema = SUITE_ONLY.schema;

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
  /**
   * The library to parse with, asked again whenever the program is set, as apps come and go.
   * Without it the suite's alone. A component of an app is run as a program of its own.
   */
  apps?: () => AppLibrary;
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

/** One use of an app's component: the program that draws it, with its own state. */
interface Instance {
  name: string;
  program: Program;
}

function isNode(value: unknown): value is UiNode {
  return isRecord(value) && typeof value.component === "string" && isRecord(value.props);
}

/** A prop of a node with the `key`s of the nodes in it under `prefix`. */
function prefixValue(value: unknown, prefix: string): unknown {
  if (Array.isArray(value)) return value.map((item) => prefixValue(item, prefix));
  if (isNode(value)) return prefixKeys(value, prefix);
  if (!isRecord(value)) return value;
  return Object.fromEntries(
    Object.entries(value).map(([name, item]) => [name, prefixValue(item, prefix)]),
  );
}

/** `key`s of a node tree under `prefix`, so what a use of a component holds can be found again. */
function prefixKeys(node: UiNode, prefix: string): UiNode {
  const props = Object.fromEntries(
    Object.entries(node.props).map(([name, item]) => [name, prefixValue(item, prefix)]),
  );
  return node.key === undefined ? { ...node, props } : { ...node, key: prefix + node.key, props };
}

/** The arguments of a call as JSON the component's program can read: data, nothing of the AST. */
function argsOf(props: Record<string, unknown>): unknown {
  return JSON.parse(
    JSON.stringify(props, (_, value: unknown) =>
      isElement(value) || isPlan(value) || isReactiveAssign(value) ? null : value,
    ),
  );
}

export class Program {
  private readonly store: Store = createStore();
  private readonly queries: QueryManager;
  private lib: AppLibrary = SUITE_ONLY;
  private parser = createStreamingParser(this.lib.schema, this.lib.library.root);
  /** The uses of app components, by where they sit in this program (`root.children.0`). */
  private readonly instances = new Map<string, Instance>();
  private touched = new Set<string>();
  private expansionErrors: UiError[] = [];
  private readonly values = new Map<string, unknown>();
  private readonly errors = new Map<string, string>();
  private readonly context: EvaluationContext;
  private source = "";
  /** The statements of the source by id, read when a query asks for one (see `plain`). */
  private statements: Map<string, ReturnType<typeof split>[number]> | undefined;
  private resolving = new Set<string>();
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
      resolveRef: (name) => this.resolve(name),
      actions: schema.actions,
    };
  }

  /** Sets the program text. `streaming` is true while more of it may still arrive. */
  set(source: string, streaming: boolean): void {
    if (source === this.source && streaming === this.streaming && this.parsed) return;
    const lib = this.env.apps?.() ?? SUITE_ONLY;
    if (lib !== this.lib) {
      this.lib = lib;
      this.parser = createStreamingParser(lib.schema, lib.library.root);
    }
    this.source = source;
    this.statements = undefined;
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

  /** What a reference names: a mutation's result, a query's, else a plain statement's value. */
  private resolve(name: string): unknown {
    const mutation = this.queries.getMutationResult(name);
    if (mutation !== undefined && mutation !== null) return mutation;
    const query = this.queries.getResult(name);
    if (query !== undefined && query !== null) return query;
    return this.plain(name);
  }

  /**
   * The value of a plain statement (`args = {…}`, `limit = 5`), for what lang-core leaves as a
   * reference: the arguments of a Query or Mutation. In an element's props it inlines them.
   */
  private plain(name: string): unknown {
    if (this.resolving.has(name)) return null;
    this.statements ??= new Map(
      split(tokenize(autoClose(this.source.trim()).text)).map((stmt) => [stmt.id, stmt]),
    );
    const statement = this.statements.get(name);
    if (!statement) return null;
    const expression = parseExpression(statement.tokens);
    // A query or a mutation is a statement too, but its value is its result, not its call.
    if (
      expression.k === "Comp" &&
      (expression.name === "Query" || expression.name === "Mutation")
    ) {
      return null;
    }
    this.resolving.add(name);
    try {
      return evaluate(expression, this.context);
    } finally {
      this.resolving.delete(name);
    }
  }

  /** The evaluated doc. */
  doc(): UiDoc {
    return this.current().doc;
  }

  dispose(): void {
    this.disposed = true;
    this.queries.dispose();
    for (const { program } of this.instances.values()) program.dispose();
    this.instances.clear();
  }

  /** A field's new value. False when there is no such field, the value isn't one it takes, or the doc is streaming. */
  input(key: string, value: unknown): boolean {
    const routed = this.route(key);
    if (routed) {
      const ok = !this.streaming && routed.program.input(routed.key, value);
      if (ok) this.rendered = undefined;
      return ok;
    }
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
    const routed = this.route(key);
    if (routed) {
      if (this.streaming) return { refused: "streaming", assistant: [] };
      try {
        return await routed.program.press(routed.key);
      } finally {
        this.rendered = undefined;
      }
    }
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

  // ─── App components ───────────────────────────────────────────────────────

  /** The use of a component a key (`<where>/<key inside it>`) belongs to, and the key in it. */
  private route(key: string): { program: Program; key: string } | undefined {
    const at = key.indexOf("/");
    if (at < 0) return undefined;
    this.current();
    const instance = this.instances.get(key.slice(0, at));
    return instance && { program: instance.program, key: key.slice(at + 1) };
  }

  /**
   * One use of an app's component, drawn: its program runs with the arguments of the call, in a
   * program of its own that is kept for as long as the use stays in the doc, so `$state`, queries
   * and keys belong to the use and two uses of one component share nothing.
   */
  private expand(
    name: string,
    id: string,
    args: Record<string, unknown>,
    partial: boolean,
    statement: string | undefined,
  ): UiNode {
    const registered = this.lib.components.get(name);
    let instance = this.instances.get(id);
    if (instance?.name !== name) {
      instance?.program.dispose();
      // Without `apps`: a component is made of the library alone.
      const program = new Program(
        `${this.id}/${id}`,
        this.env.callTool
          ? { markdown: this.env.markdown, callTool: this.env.callTool }
          : { markdown: this.env.markdown },
      );
      program.onChange = () => this.later();
      instance = { name, program };
      this.instances.set(id, instance);
    }
    this.touched.add(id);
    const failed = (why: string): UiNode => {
      const text = `${name} could not be drawn: ${why}`;
      return {
        component: "Callout",
        props: { text, tone: "danger", blocks: this.env.markdown(text) },
      };
    };
    if (!registered) return failed("no app registers it");
    instance.program.set(
      instanceProgram(registered.component.openui, argsOf(args)),
      this.streaming,
    );
    const doc = instance.program.doc();
    for (const error of doc.errors ?? []) {
      const own: UiError = { code: error.code, message: `${name}: ${error.message}` };
      if (statement !== undefined) own.statementId = statement;
      this.expansionErrors.push(own);
    }
    if (!doc.root) return failed(doc.errors?.[0]?.message ?? "its program drew nothing");
    const root = prefixKeys(doc.root, `${id}/`);
    return partial ? { ...root, partial: true } : root;
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

  /** What a field holds: its $variable's value, else what was typed, else the program's. */
  private currentRaw(field: Field): unknown {
    if (field.bound !== undefined) return this.store.get(field.bound);
    return this.values.get(field.id) ?? field.literal;
  }

  /** The names a CheckBoxGroup has checked, in the items' order; the items' own start values until it holds a list. */
  private checkedNames(field: Field, raw: unknown): string[] {
    let picked: unknown[] = [];
    if (Array.isArray(raw)) picked = raw;
    else if (raw === null || raw === undefined) picked = field.checked ?? [];
    return (field.options ?? []).filter((name) => picked.includes(name));
  }

  /** Every switch of a SwitchGroup on or off; the items' own start values for those it doesn't hold. */
  private switchStates(field: Field, raw: unknown): Record<string, boolean> {
    const held = isRecord(raw) ? raw : undefined;
    const states: Record<string, boolean> = {};
    for (const name of field.options ?? []) {
      const state = held?.[name];
      states[name] = typeof state === "boolean" ? state : (field.checked ?? []).includes(name);
    }
    return states;
  }

  private valueOf(field: Field): unknown {
    const raw = this.currentRaw(field);
    switch (field.component) {
      case "CheckBox":
      case "Switch":
        return raw === true;
      case "CheckBoxGroup":
        return this.checkedNames(field, raw);
      case "SwitchGroup":
        return this.switchStates(field, raw);
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
      case "CheckBoxGroup": {
        const names = field.options ?? [];
        if (!Array.isArray(value) || !value.every((v) => names.includes(String(v))))
          return undefined;
        return names.filter((name) => value.includes(name));
      }
      case "SwitchGroup": {
        const names = field.options ?? [];
        if (!isRecord(value)) return undefined;
        const changes = Object.entries(value);
        if (!changes.every(([k, v]) => names.includes(k) && typeof v === "boolean"))
          return undefined;
        return { ...this.switchStates(field, this.currentRaw(field)), ...value };
      }
      case "Select":
      case "RadioGroup":
      case "OptionCards":
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
    this.touched = new Set();
    this.expansionErrors = [];
    // Tools run for a finished program only; a streaming one is still being written.
    if (!this.streaming) this.startQueries(parsed);
    const evalErrors: OpenUIError[] = [];
    let root: ElementNode | null = null;
    try {
      root = evaluateRoot(parsed, {
        ctx: this.context,
        library: this.lib.library,
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
      isAppComponent: (name) => this.lib.components.has(name),
      expand: (name, id, args, partial, statement) =>
        this.expand(name, id, args, partial, statement),
    });
    if (root) doc.root = converter.convert(root);
    // A use that is gone takes its state with it.
    for (const [id, instance] of this.instances) {
      if (this.touched.has(id)) continue;
      instance.program.dispose();
      this.instances.delete(id);
    }
    if (this.streaming) {
      // What looks broken may only be unfinished: errors wait for the end.
      doc.streaming = true;
    } else {
      const errors = [
        ...parsed.meta.errors.filter((error) => error.severity !== "warning").map(uiError),
        ...evalErrors.map(uiError),
        ...this.expansionErrors,
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
