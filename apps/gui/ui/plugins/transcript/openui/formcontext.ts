// What a field needs of the form or control around it.
import { getContext, setContext } from "svelte";
import type { UiNode } from "@neoworks-dev/neo-protocol/types";

const FORM = Symbol("openui form");
const CONTROL = Symbol("openui control");

export function provideForm(form: () => UiNode): void {
  setContext(FORM, form);
}

/** The Form around the field, if any: Enter presses its first button. */
export function useForm(): () => UiNode | undefined {
  return getContext<(() => UiNode) | undefined>(FORM) ?? (() => undefined);
}

export function provideControl(): void {
  setContext(CONTROL, true);
}

/** Whether a FormControl draws the field's label and error. */
export function inControl(): boolean {
  return getContext<boolean | undefined>(CONTROL) === true;
}
