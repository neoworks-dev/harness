// The neoui docs of one session view: a running Program for each doc of each item, kept for as
// long as the view lives so typed values and variables survive the item being re-sent
// (docs/neoui.md). A Program parses, evaluates and handles input; this keeps them and renders
// templates written in neoui.
import { type AppLibrary, type CallTool, Program } from "@neoworks/neoui";
import type { UiDoc } from "@neoworks-dev/neo-protocol/types";
import { parseMarkdown } from "./markdown.ts";

/** A template that is a program: `{"neoui": "<markup>"}`, in place of component JSON. */
export function isNeoUiTemplate(template: unknown): template is { neoui: string } {
  return (
    typeof template === "object" &&
    template !== null &&
    "neoui" in template &&
    typeof template.neoui === "string"
  );
}

/** What a template reads the tool's result as: `data`, absent when the tool returned nothing. */
export function templateData(data: unknown): Record<string, unknown> {
  if (data === undefined) return {};
  return { data };
}

export class NeoUiDocs {
  private readonly programs = new Map<string, Program>();

  /**
   * `changed` is told of an item whose doc changed on its own (a query answered), `tools` runs
   * the tools Query and Mutation name, `apps` gives the library with the components of the apps
   * (asked whenever a program is set, as apps come and go).
   */
  constructor(
    private readonly changed: (ref: string) => void,
    private readonly tools?: CallTool,
    private readonly apps?: () => AppLibrary,
    /** Whether someone has the view open: a query that polls only asks while this holds. */
    private readonly watched?: () => boolean,
  ) {}

  /** Stops the timers of every program: the view is gone. */
  dispose(): void {
    for (const program of this.programs.values()) program.dispose();
    this.programs.clear();
  }

  private static id(ref: string, doc: string): string {
    return `${ref}\0${doc}`;
  }

  /**
   * Doc `doc` of item `ref` as the program says; the program is kept for the next call.
   * `provided` names values for its expressions (a template reads the tool's result as `data`).
   */
  doc(
    ref: string,
    doc: string,
    source: string,
    streaming: boolean,
    provided?: Record<string, unknown>,
  ): UiDoc {
    const id = NeoUiDocs.id(ref, doc);
    let program = this.programs.get(id);
    if (!program) {
      program = new Program(doc, {
        markdown: (text) => parseMarkdown(text),
        ...(this.tools && { callTool: this.tools }),
        ...(this.apps && { apps: this.apps }),
        ...(this.watched && { watched: this.watched }),
      });
      program.onChange = () => this.changed(ref);
      this.programs.set(id, program);
    }
    // The view must survive whatever a program does to the evaluator.
    try {
      if (provided) program.provide(provided);
      program.set(source, streaming);
      return program.doc();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      return { id: doc, errors: [{ code: "render-error", message }] };
    }
  }

  program(ref: string, doc: string): Program | undefined {
    return this.programs.get(NeoUiDocs.id(ref, doc));
  }
}
