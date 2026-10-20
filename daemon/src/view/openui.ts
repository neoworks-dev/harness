// The OpenUI docs of one session view: a running Program for each doc of each item, kept for as
// long as the view lives so typed values and $variables survive the item being re-sent
// (docs/openui.md). A Program parses, evaluates and handles input; this keeps them and renders
// templates written in OpenUI Lang.
import { Program, type CallTool } from "@suite/openui/program";
import type { UiDoc } from "@suite/protocol/types";
import { parseMarkdown } from "./markdown.ts";

/** A template that is a program: `{"openui": "<program>"}`, in place of component JSON. */
export function isOpenUiTemplate(template: unknown): template is { openui: string } {
  return (
    typeof template === "object" &&
    template !== null &&
    "openui" in template &&
    typeof template.openui === "string"
  );
}

/** The program of a template: its text, then the tool's data as the last statement. */
export function templateProgram(template: { openui: string }, data: unknown): string {
  const source = template.openui.replace(/\n+$/, "");
  // JSON is OpenUI Lang for a value. A later statement wins, so the template can't shadow it.
  return data === undefined ? source : `${source}\ndata = ${JSON.stringify(data)}`;
}

export class OpenUiDocs {
  private readonly programs = new Map<string, Program>();

  /**
   * `changed` is told of an item whose doc changed on its own (a query answered), `tools` runs
   * the tools Query and Mutation name.
   */
  constructor(
    private readonly changed: (ref: string) => void,
    private readonly tools?: CallTool,
  ) {}

  private static id(ref: string, doc: string): string {
    return `${ref}\0${doc}`;
  }

  /** Doc `doc` of item `ref` as the program says; the program is kept for the next call. */
  doc(ref: string, doc: string, source: string, streaming: boolean): UiDoc {
    const id = OpenUiDocs.id(ref, doc);
    let program = this.programs.get(id);
    if (!program) {
      program = new Program(doc, {
        markdown: (text) => parseMarkdown(text),
        ...(this.tools && { callTool: this.tools }),
      });
      program.onChange = () => this.changed(ref);
      this.programs.set(id, program);
    }
    // The view must survive whatever a program does to the evaluator.
    try {
      program.set(source, streaming);
      return program.doc();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      return { id: doc, errors: [{ code: "render-error", message }] };
    }
  }

  program(ref: string, doc: string): Program | undefined {
    return this.programs.get(OpenUiDocs.id(ref, doc));
  }
}
