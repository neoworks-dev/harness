import { expect, test } from "bun:test";
import { Keymap, chord, type KeyEvent } from "../ui/kernel/keymap.ts";

function press(key: string, held: Partial<KeyEvent> = {}): KeyEvent & { prevented: boolean } {
  const event = {
    key,
    ctrlKey: false,
    altKey: false,
    metaKey: false,
    shiftKey: false,
    defaultPrevented: false,
    prevented: false,
    preventDefault() {
      event.prevented = true;
      event.defaultPrevented = true;
    },
    ...held,
  };
  return event;
}

test("a chord names the modifiers held and the key", () => {
  expect(chord(press("s", { ctrlKey: true }))).toBe("ctrl-s");
  expect(chord(press("Tab", { shiftKey: true }))).toBe("shift-tab");
  expect(chord(press("Escape"))).toBe("escape");
  expect(chord(press("F6"))).toBe("f6");
  expect(chord(press("S", { ctrlKey: true, shiftKey: true }))).toBe("ctrl-shift-s");
  expect(chord(press(" "))).toBe("space");
  expect(chord(press("Control", { ctrlKey: true }))).toBeUndefined();
  expect(chord(press("Shift", { shiftKey: true }))).toBeUndefined();
});

test("a used key is kept from the focused input; an unbound one is left alone", () => {
  const keymap = new Keymap();
  let hits = 0;
  keymap.bind("ctrl-s", () => void (hits += 1));
  const bound = press("s", { ctrlKey: true });
  expect(keymap.dispatch(bound)).toBe(true);
  expect(bound.prevented).toBe(true);
  const unbound = press("q", { ctrlKey: true });
  expect(keymap.dispatch(unbound)).toBe(false);
  expect(unbound.prevented).toBe(false);
  expect(hits).toBe(1);
});

test("the newest binding goes first, and false passes the key on", () => {
  const keymap = new Keymap();
  const seen: string[] = [];
  keymap.bind("escape", () => void seen.push("old"));
  keymap.bind("escape", () => {
    seen.push("new");
    return false;
  });
  const event = press("Escape");
  keymap.dispatch(event);
  expect(seen).toEqual(["new", "old"]);
  expect(event.prevented).toBe(true);

  const unused = new Keymap();
  unused.bind("escape", () => false);
  const kept = press("Escape");
  expect(unused.dispatch(kept)).toBe(false);
  expect(kept.prevented).toBe(false);
});

test("a disposer removes the binding", () => {
  const keymap = new Keymap();
  let hits = 0;
  const unbind = keymap.bind("f6", () => void (hits += 1));
  keymap.dispatch(press("F6"));
  unbind();
  expect(keymap.dispatch(press("F6"))).toBe(false);
  expect(hits).toBe(1);
});

test("observers hear every key but a bare modifier, first, and never use it", () => {
  const keymap = new Keymap();
  const order: string[] = [];
  keymap.observe((event) => void order.push(`saw ${event.key}`));
  keymap.bind("a", () => void order.push("bound"));
  const stray = press("z");
  keymap.dispatch(stray);
  keymap.dispatch(press("a"));
  keymap.dispatch(press("Shift", { shiftKey: true }));
  expect(order).toEqual(["saw z", "saw a", "bound"]);
  expect(stray.prevented).toBe(false);
});
