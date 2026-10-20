// App components: what an app registers in `app.presence` as `components`, built from the library
// for the agent to use (docs/openui.md). An app component is a name, a description, typed
// parameters and a program over the suite@1 library; the daemon adds it to the library its
// sessions parse with, so lang-core checks the agent's calls, and runs the program with the
// arguments in place of each call (`Program`). Renderers never see one.
import {
  createParser,
  defineComponent,
  type DefinedComponent,
  type Library,
} from "@openuidev/lang-core";
import type { AppComponent } from "@suite/protocol/types";
import { z } from "zod/v4";
import { createSuiteLibrary, library as suite } from "./library.ts";

/** The components of each app, by app name and then component name. */
export type AppComponents = Record<string, Record<string, AppComponent>>;

/** An app's component as the library has it. */
export interface Registered {
  app: string;
  /** The name the app registered it under. */
  name: string;
  /** The name to call it by: `name` when the app has it, else `qualified`. */
  call: string;
  /** The app's name in PascalCase, then `name`: always this component's. */
  qualified: string;
  component: AppComponent;
  /** The parameter names, in the order the arguments come. */
  params: string[];
}

/** The suite library with the components of some apps added, and where each came from. */
export interface AppLibrary {
  library: Library;
  schema: ReturnType<Library["toJSONSchema"]>;
  components: ReadonlyMap<string, Registered>;
}

/**
 * A component's name is an identifier lang-core reads as a call: lang-core takes `photo-editor.X`
 * for a subtraction and a member access, so the app goes in front of the name instead
 * (`PhotoEditorBrightnessCard`, see `qualifiedName`).
 */
const NAME = /^[A-Z][A-Za-z0-9]*$/;
const PARAM = /^[a-z][A-Za-z0-9]*$/;
const TYPE = /^(string|number|boolean|any)(\[\])?(\?)?$/;

/** What the language itself calls by a name. */
const RESERVED: ReadonlySet<string> = new Set(["Query", "Mutation", "Action"]);

const parser = createParser(suite.toJSONSchema(), suite.root);

/** The zod type of a param's `string`, `number[]`, `any?`… */
function paramType(spec: string): z.ZodType {
  const [, base = "any", list, optional] = TYPE.exec(spec) ?? [];
  let type: z.ZodType;
  switch (base) {
    case "string":
      type = z.string();
      break;
    case "number":
      type = z.number();
      break;
    case "boolean":
      type = z.boolean();
      break;
    default:
      type = z.any();
  }
  if (list !== undefined) type = z.array(type);
  return optional === undefined ? type : type.optional();
}

/** `asset: string, name?: string`, as the prompt shows the arguments of a component. */
export function paramList(params: Record<string, string>): string {
  return Object.entries(params)
    .map(([name, spec]) =>
      spec.endsWith("?") ? `${name}?: ${spec.slice(0, -1)}` : `${name}: ${spec}`,
    )
    .join(", ");
}

/** The program of one use: the component's, then the arguments as its last statement. */
export function instanceProgram(source: string, args: unknown): string {
  // A later statement wins, so the program can't shadow its arguments.
  return `${source.replace(/\n+$/, "")}\nargs = ${JSON.stringify(args)}`;
}

/** `photo-editor` + `BrightnessCard` → `PhotoEditorBrightnessCard`: a component's own name. */
export function qualifiedName(app: string, name: string): string {
  const prefix = app
    .split(/[^A-Za-z0-9]+/)
    .filter((part) => part !== "")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join("");
  return `${prefix}${name}`;
}

/**
 * The components `app` registers that can be used, and what is wrong with the others: a name that
 * is not a name, a param that is not typed or follows an optional one, a program that does not
 * parse. One bad component never costs the app the rest. A name the library, the language or
 * another app already has is fine: the component is then called by its qualified name.
 */
export function checkComponents(components: Record<string, AppComponent>): {
  usable: Record<string, AppComponent>;
  problems: string[];
} {
  const usable: Record<string, AppComponent> = {};
  const problems: string[] = [];
  for (const [name, component] of Object.entries(components)) {
    const before = problems.length;
    const wrong = (why: string): void => {
      problems.push(`component ${name}: ${why}`);
    };
    if (!NAME.test(name)) {
      wrong(
        "a name is letters and digits starting with a capital (PascalCase), as lang-core reads it as a call",
      );
    }
    if (component.description.trim() === "") wrong("it needs a description for the agent");
    let optional = false;
    for (const [param, spec] of Object.entries(component.params)) {
      if (!PARAM.test(param)) wrong(`param ${param}: a name is camelCase letters and digits`);
      else if (!TYPE.test(spec)) {
        wrong(`param ${param}: ${spec} is not string, number, boolean or any, with [] and ? after`);
      } else if (spec.endsWith("?")) optional = true;
      else if (optional) wrong(`param ${param}: required params come before optional ones`);
    }
    const parsed = parser.parse(instanceProgram(component.openui, {}));
    const broken = parsed.meta.errors.find((error) => error.severity !== "warning");
    if (broken) wrong(`its program: ${broken.message}`);
    else if (parsed.root === null) wrong("its program has no root statement");
    else if (parsed.meta.unresolved.length > 0) {
      wrong(`its program uses ${parsed.meta.unresolved.join(", ")} without defining it`);
    }
    if (problems.length === before) usable[name] = component;
  }
  return { usable, problems };
}

const taken = (name: string): boolean => name in suite.components || RESERVED.has(name);

/**
 * The suite library with these apps' components, each a call lang-core checks against its
 * params, under its qualified name and, when nothing else has it, its plain one. `apps` comes in
 * the order the apps first registered: an app keeps a plain name a later one also registers, so
 * what a name draws never changes under an old reply.
 */
export function appLibrary(apps: AppComponents): AppLibrary {
  const components = new Map<string, Registered>();
  const defined: DefinedComponent[] = [];
  const define = (call: string, registered: Registered): void => {
    const { component } = registered;
    const shape = Object.fromEntries(
      Object.entries(component.params).map(([param, spec]) => [param, paramType(spec)]),
    );
    defined.push(
      defineComponent({
        name: call,
        props: z.object(shape),
        description: component.description,
        component: null,
      }),
    );
    components.set(call, registered);
  };
  // Qualified names first, so no plain name can take one.
  const all: Registered[] = [];
  for (const [app, theirs] of Object.entries(apps)) {
    for (const [name, component] of Object.entries(theirs)) {
      const qualified = qualifiedName(app, name);
      if (taken(qualified) || components.has(qualified)) continue;
      const registered: Registered = {
        app,
        name,
        call: qualified,
        qualified,
        component,
        params: Object.keys(component.params),
      };
      define(qualified, registered);
      all.push(registered);
    }
  }
  for (const registered of all) {
    if (taken(registered.name) || components.has(registered.name)) continue;
    registered.call = registered.name;
    define(registered.name, registered);
  }
  const library = createSuiteLibrary(defined);
  return { library, schema: library.toJSONSchema(), components };
}

/** Each app's components as the agent calls them, by app: `{ "photo-editor": ["BrightnessCard"] }`. */
export function callNames(lib: AppLibrary): Record<string, string[]> {
  const out: Record<string, string[]> = {};
  for (const [call, registered] of lib.components) {
    if (call === registered.call) (out[registered.app] ??= []).push(call);
  }
  return out;
}

/** The suite library alone. */
export const SUITE_ONLY: AppLibrary = appLibrary({});
