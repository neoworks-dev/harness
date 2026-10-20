// An evaluated ElementNode tree as the wire's UiNode tree (docs/openui.md): concrete props,
// markdown parsed, keys on the nodes a user can act on, and a note of each field and button for
// openui.input and openui.press to find again.
import {
  ACTION_STEPS,
  isReactiveAssign,
  parseStructuredRules,
  type ActionPlan,
  type ElementNode,
  type ParsedRule,
} from "@openuidev/lang-core";
import type { Block, UiNode } from "@suite/protocol/types";
import { INPUTS } from "./library.ts";

/** An input of the doc, as the last render found it. */
export interface Field {
  key: string;
  /** Identifies the value: the form it is in plus its name. */
  id: string;
  component: string;
  name: string;
  form?: string;
  rules: ParsedRule[];
  /** The `$variable` its value is bound to. */
  bound?: string;
  /** A fixed `value` the program gave instead of a binding: the start value. */
  literal?: unknown;
  options?: string[];
  /** The groups' items that start checked. */
  checked?: string[];
  min?: number;
  max?: number;
}

export interface ButtonInfo {
  key: string;
  label: string;
  form?: string;
  plan: ActionPlan;
}

/** What the converter asks of the program it converts for. */
export interface ConvertHost {
  isPending(statement: string | undefined): boolean;
  value(field: Field): unknown;
  error(field: Field): string | undefined;
  mutationLoading(statementId: string): boolean;
  markdown(source: string): Block[];
  /** Whether the name is an app's component, which is drawn by running its program. */
  isAppComponent(name: string): boolean;
  /** The drawn use of an app's component: `id` is where it sits, `args` its evaluated arguments. */
  expand(
    name: string,
    id: string,
    args: Record<string, unknown>,
    partial: boolean,
    statement: string | undefined,
  ): UiNode;
}

/** Where a node sits: the statement it belongs to and the path from that statement's component. */
interface Place {
  statement: string | undefined;
  path: (string | number)[];
  form: string | undefined;
}

const SAFE_URL = /^(https?:|mailto:)/i;

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isElement(value: unknown): value is ElementNode {
  return isRecord(value) && value.type === "element" && typeof value.typeName === "string";
}

export function isPlan(value: unknown): value is ActionPlan {
  return isRecord(value) && Array.isArray(value.steps);
}

/** A prop that should be text: itself, else nothing. */
function text(value: unknown): string {
  if (typeof value === "string") return value;
  return "";
}

/** Nodes that run a plan when pressed: a button, a follow-up suggestion. */
function pressable(component: string): boolean {
  return component === "Button" || component === "FollowUpItem";
}

function interactive(component: string): boolean {
  return INPUTS.has(component) || pressable(component);
}

/** The interactive nodes of each statement, counted. */
function countInteractive(
  value: unknown,
  statement: string | undefined,
  counts: Map<string | undefined, number>,
): void {
  if (Array.isArray(value)) {
    for (const item of value) countInteractive(item, statement, counts);
  } else if (isElement(value)) {
    const own = value.statementId ?? statement;
    if (interactive(value.typeName)) counts.set(own, (counts.get(own) ?? 0) + 1);
    for (const prop of Object.values(value.props)) countInteractive(prop, own, counts);
  } else if (isRecord(value) && !isPlan(value) && !isReactiveAssign(value)) {
    for (const prop of Object.values(value)) countInteractive(prop, statement, counts);
  }
}

/** The items of a CheckBoxGroup or SwitchGroup: their names and which start on. */
function groupItems(items: unknown): { names: string[]; checked: string[] } {
  const out = { names: [] as string[], checked: [] as string[] };
  if (!Array.isArray(items)) return out;
  for (const item of items) {
    if (!isElement(item) || typeof item.props.name !== "string") continue;
    out.names.push(item.props.name);
    if (item.props.checked === true) out.checked.push(item.props.name);
  }
  return out;
}

function optionValues(options: unknown): string[] {
  if (!Array.isArray(options)) return [];
  const values: string[] = [];
  for (const option of options) {
    if (isElement(option) && typeof option.props.value === "string") {
      values.push(option.props.value);
    }
  }
  return values;
}

export class Converter {
  readonly fields = new Map<string, Field>();
  readonly buttons = new Map<string, ButtonInfo>();
  private readonly counts = new Map<string | undefined, number>();

  constructor(private readonly host: ConvertHost) {}

  convert(root: ElementNode): UiNode {
    countInteractive(root, root.statementId, this.counts);
    return this.node(root, { statement: root.statementId, path: [], form: undefined });
  }

  /**
   * The key of an interactive node: its statement's id, or the id plus the path to the node when
   * the statement has more than one, or the node comes from a list (a button of `Buttons([…])`).
   */
  private key(place: Place): string {
    const statement = place.statement ?? "root";
    const alone = (this.counts.get(place.statement) ?? 0) === 1;
    const listed = place.path.some((step) => typeof step === "number");
    if (alone && !listed) return statement;
    return [statement, ...place.path].join(".");
  }

  private node(el: ElementNode, at: Place): UiNode {
    const place: Place =
      el.statementId === undefined ? at : { ...at, statement: el.statementId, path: [] };
    if (this.host.isAppComponent(el.typeName)) {
      // Where it sits names the use; the same wherever the program is re-evaluated from.
      const id = [place.statement ?? "root", ...place.path].join(".");
      const partial = el.partial && this.host.isPending(place.statement);
      return this.host.expand(el.typeName, id, el.props, partial, place.statement);
    }
    const form =
      el.typeName === "Form" && typeof el.props.name === "string" ? el.props.name : at.form;
    const props: Record<string, unknown> = {};
    for (const [name, value] of Object.entries(el.props)) {
      // A button's action is what the daemon runs; an input's value is what it holds.
      if (name === "action" && el.typeName === "Button") continue;
      if (name === "value" && INPUTS.has(el.typeName)) continue;
      const converted = this.value(value, { ...place, path: [...place.path, name], form });
      if (converted !== null && converted !== undefined) props[name] = converted;
    }
    const key = interactive(el.typeName) ? this.key(place) : undefined;
    if (key !== undefined && INPUTS.has(el.typeName)) this.input(el, key, at.form, props);
    if (key !== undefined && pressable(el.typeName)) this.button(el, key, at.form, props);
    if (
      (el.typeName === "Text" || el.typeName === "Callout") &&
      typeof el.props.text === "string"
    ) {
      props.blocks = this.host.markdown(el.props.text);
    }
    const partial = el.partial && this.host.isPending(place.statement);
    // Written in the order the fixtures use: component, key, partial, props.
    return {
      component: el.typeName,
      ...(key !== undefined && { key }),
      ...(partial && { partial: true }),
      props,
    };
  }

  private value(value: unknown, place: Place): unknown {
    if (Array.isArray(value)) {
      return value.map((item, i) => this.value(item, { ...place, path: [...place.path, i] }));
    }
    if (isElement(value)) return this.node(value, place);
    if (isReactiveAssign(value) || isPlan(value)) return null;
    if (isRecord(value)) {
      const out: Record<string, unknown> = {};
      for (const [name, item] of Object.entries(value)) {
        const converted = this.value(item, { ...place, path: [...place.path, name] });
        if (converted !== null && converted !== undefined) out[name] = converted;
      }
      return out;
    }
    return value;
  }

  private input(
    el: ElementNode,
    key: string,
    form: string | undefined,
    props: Record<string, unknown>,
  ): void {
    const name = typeof el.props.name === "string" ? el.props.name : key;
    const rules = parseStructuredRules(el.props.rules);
    const binding = el.props.value;
    const field: Field = { key, id: `${form ?? ""}\0${name}`, component: el.typeName, name, rules };
    if (form !== undefined) field.form = form;
    if (isReactiveAssign(binding)) field.bound = binding.target;
    else if (binding !== null && binding !== undefined) field.literal = binding;
    if (el.typeName === "Select" || el.typeName === "RadioGroup" || el.typeName === "OptionCards") {
      field.options = optionValues(el.props.options);
    }
    if (el.typeName === "CheckBoxGroup" || el.typeName === "SwitchGroup") {
      const { names, checked } = groupItems(el.props.items);
      field.options = names;
      field.checked = checked;
    }
    if (el.typeName === "Slider") {
      if (typeof el.props.min === "number") field.min = el.props.min;
      if (typeof el.props.max === "number") field.max = el.props.max;
    }
    this.fields.set(key, field);
    if (rules.some((rule) => rule.type === "required")) props.required = true;
    const value = this.host.value(field);
    if (value !== undefined) props.value = value;
    const error = this.host.error(field);
    if (error !== undefined) props.error = error;
  }

  private button(
    el: ElementNode,
    key: string,
    form: string | undefined,
    props: Record<string, unknown>,
  ): void {
    // A follow-up suggestion is a button whose label is its text.
    const label = text(el.typeName === "Button" ? el.props.label : el.props.text);
    // Without an action a button sends its label to the assistant.
    const plan: ActionPlan = isPlan(el.props.action)
      ? el.props.action
      : { steps: [{ type: ACTION_STEPS.ToAssistant, message: label }] };
    const info: ButtonInfo = { key, label, plan };
    if (form !== undefined) info.form = form;
    this.buttons.set(key, info);
    // A button that only opens a link or copies text is the client's to press.
    const only = plan.steps.length === 1 ? plan.steps[0] : undefined;
    if (only?.type === ACTION_STEPS.OpenUrl && SAFE_URL.test(only.url)) props.url = only.url;
    if (only?.type === "custom_action" && only.name === "Copy") {
      const text = only.params.text;
      if (typeof text === "string") props.copy = text;
    }
    const loading = plan.steps.some(
      (step) =>
        step.type === ACTION_STEPS.Run &&
        step.refType === "mutation" &&
        this.host.mutationLoading(step.statementId),
    );
    if (loading) props.disabled = true;
  }
}
