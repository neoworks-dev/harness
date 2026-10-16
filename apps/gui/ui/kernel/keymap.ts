// The window's keys, as data: plugins bind chords and the window hands every key press to
// `dispatch`. Plain code, so it runs in tests; `Keys.svelte` is the window half and the `keymap`
// service the plugin half.

/** What a handler needs of a key press (a `KeyboardEvent` has all of it). */
export interface KeyEvent {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  defaultPrevented: boolean;
  preventDefault(): void;
}

/** Returning `false` says the key was not used, so the next binding of the chord gets it. */
export type KeyHandler = (event: KeyEvent) => boolean | void;

type Dispose = () => void;

const MODIFIER_KEYS = new Set(["control", "shift", "alt", "meta"]);

/**
 * The chord a press makes, such as `ctrl-s`, `shift-tab`, `escape` or `f6`: the modifiers held
 * (in the order ctrl, alt, meta, shift) and the key, lower case. A modifier on its own is no chord.
 */
export function chord(event: KeyEvent): string | undefined {
  const key = event.key === " " ? "space" : event.key.toLowerCase();
  if (MODIFIER_KEYS.has(key)) return undefined;
  const held = [
    event.ctrlKey && "ctrl",
    event.altKey && "alt",
    event.metaKey && "meta",
    event.shiftKey && "shift",
  ].filter(Boolean);
  return [...held, key].join("-");
}

export class Keymap {
  private readonly bindings = new Map<string, KeyHandler[]>();
  private readonly observers = new Set<KeyHandler>();

  /** Binds a chord. The newest binding of a chord gets the key first. */
  bind(chord: string, run: KeyHandler): Dispose {
    const list = this.bindings.get(chord) ?? [];
    this.bindings.set(chord, [run, ...list]);
    return () => {
      const rest = (this.bindings.get(chord) ?? []).filter((handler) => handler !== run);
      if (rest.length > 0) this.bindings.set(chord, rest);
      else this.bindings.delete(chord);
    };
  }

  /** Hears every key press (not bare modifiers) before the bindings do, and never uses it. */
  observe(run: KeyHandler): Dispose {
    this.observers.add(run);
    return () => this.observers.delete(run);
  }

  /** Runs the bindings of the press's chord until one uses it, and keeps the focused input from typing it. */
  dispatch(event: KeyEvent): boolean {
    const pressed = chord(event);
    if (pressed === undefined) return false;
    for (const observer of [...this.observers]) observer(event);
    for (const run of [...(this.bindings.get(pressed) ?? [])]) {
      if (run(event) === false) continue;
      event.preventDefault();
      return true;
    }
    return false;
  }
}
