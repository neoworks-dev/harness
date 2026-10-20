// How the parts of a doc reach the doc: its session (values, what is sent, the visual state).
import { getContext, setContext } from "svelte";
import type { DocSession } from "./state.svelte.ts";

const KEY = Symbol("openui doc");

export function provideDoc(session: DocSession): void {
  setContext(KEY, session);
}

/** The doc the component is drawn in (call it while initialising). */
export function useDoc(): DocSession {
  const session = getContext<DocSession | undefined>(KEY);
  if (!session) throw new Error("useDoc: this component was not drawn inside an OpenUI doc");
  return session;
}
