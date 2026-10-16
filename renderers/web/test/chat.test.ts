import { describe, expect, test } from "bun:test";
import type { Block, Component, Theme, ViewItem, ViewPatch } from "@suite/protocol/types";
import theme from "../../../protocol/themes/default.json";
import { mountChat, type Chat, type ChatOptions } from "../src/render.ts";
import { chatStyles, themeVariables } from "../src/styles.ts";
import { newPage, type Page } from "./dom.ts";

interface Action {
  ref: string;
  action: string;
  args?: unknown;
}

interface Rig {
  page: Page;
  chat: Chat;
  actions: Action[];
  copied: string[];
  errors: Error[];
}

function rig(options: ChatOptions = {}): Rig {
  const page = newPage();
  const actions: Action[] = [];
  const copied: string[] = [];
  const errors: Error[] = [];
  const chat = mountChat(page.container, {
    theme,
    onAction: (ref, action, args) => actions.push({ ref, action, args }),
    clipboard: {
      writeText: (text) => {
        copied.push(text);
        return Promise.resolve();
      },
    },
    onError: (error) => errors.push(error),
    ...options,
  });
  return { page, chat, actions, copied, errors };
}

function para(text: string): Block {
  return { type: "paragraph", spans: [{ text }] };
}

function text(...blocks: Block[]): Component {
  return { type: "text", blocks };
}

function agent(ref: string, ...blocks: Block[]): ViewItem {
  return { ref, kind: "agent", component: text(...blocks) };
}

function withButtons(ref: string, ...keys: (string | undefined)[]): ViewItem {
  const buttons: Component[] = keys.map((key, i) => {
    const button: Component = {
      type: "button",
      label: `B${i}`,
      action: `app.act${i}`,
      args: { i },
    };
    if (key !== undefined) button.key = key;
    return button;
  });
  return {
    ref,
    kind: "tool",
    label: "tool",
    state: "done",
    component: { type: "row", children: buttons },
  };
}

function patch(seq: number, ...ops: ViewPatch["ops"]): ViewPatch {
  return { session: "s1", seq, ops };
}

function itemEl(page: Page, ref: string): HTMLElement {
  const found = page.container.querySelector<HTMLElement>(`[data-ref="${ref}"]`);
  if (!found) throw new Error(`no element for ${ref}`);
  return found;
}

function refs(page: Page): (string | null)[] {
  return Array.from(page.container.children).map((c) => c.getAttribute("data-ref"));
}

function buttonOf(item: Element, index: number): HTMLElement {
  const found = item.querySelector<HTMLElement>(`[data-button="${index}"]`);
  if (!found) throw new Error(`no button ${index}`);
  return found;
}

function press(
  page: Page,
  target: Element,
  key: string,
  init: { ctrlKey?: boolean } = {},
): boolean {
  const event = new page.window.KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
    ...init,
  });
  return target.dispatchEvent(event as unknown as Event);
}

function click(page: Page, target: Element): void {
  target.dispatchEvent(new page.window.MouseEvent("click", { bubbles: true }) as unknown as Event);
}

const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

describe("patches update only what they touch", () => {
  test("set appends a new item and replaces an existing one in place", () => {
    const { page, chat } = rig();
    chat.setItems([agent("a", para("one")), agent("b", para("two"))]);
    const a = itemEl(page, "a");
    const b = itemEl(page, "b");

    chat.applyPatch(patch(1, { op: "set", item: agent("c", para("three")) }));
    expect(refs(page)).toEqual(["a", "b", "c"]);
    expect(itemEl(page, "a")).toBe(a);

    chat.applyPatch(patch(2, { op: "set", item: agent("b", para("TWO")) }));
    expect(refs(page)).toEqual(["a", "b", "c"]);
    expect(itemEl(page, "a")).toBe(a);
    expect(itemEl(page, "b")).not.toBe(b);
    expect(itemEl(page, "b").textContent).toContain("TWO");
  });

  test("remove drops just that element", () => {
    const { page, chat } = rig();
    chat.setItems([agent("a"), agent("b"), agent("c")]);
    const c = itemEl(page, "c");
    chat.applyPatch(patch(1, { op: "remove", ref: "b" }, { op: "remove", ref: "ghost" }));
    expect(refs(page)).toEqual(["a", "c"]);
    expect(itemEl(page, "c")).toBe(c);
    expect(chat.items.map((i) => i.ref)).toEqual(["a", "c"]);
  });

  test("streaming blocks re-render only the tail of the text component", () => {
    const { page, chat } = rig();
    chat.setItems([agent("m", para("first"), para("sec"))]);
    const [first, second] = Array.from(itemEl(page, "m").querySelectorAll(".suite-text > p"));

    chat.applyPatch(
      patch(1, {
        op: "blocks",
        ref: "m",
        path: [],
        from: 1,
        blocks: [para("second"), para("third")],
      }),
    );
    const paragraphs = Array.from(itemEl(page, "m").querySelectorAll(".suite-text > p"));
    expect(paragraphs.map((p) => p.textContent)).toEqual(["first", "second", "third"]);
    expect(paragraphs[0]).toBe(first);
    expect(paragraphs[1]).not.toBe(second);

    chat.applyPatch(
      patch(2, { op: "blocks", ref: "m", path: [], from: 2, blocks: [para("third!")] }),
    );
    const after = Array.from(itemEl(page, "m").querySelectorAll(".suite-text > p"));
    expect(after.map((p) => p.textContent)).toEqual(["first", "second", "third!"]);
    expect(after[0]).toBe(first);
    expect(after[1]).toBe(paragraphs[1]);
  });

  test("blocks with a nested path reach the text inside containers", () => {
    const { page, chat } = rig();
    const item: ViewItem = {
      ref: "t",
      kind: "tool",
      component: {
        type: "card",
        title: "Card",
        children: [
          text(para("zero")),
          { type: "stack", children: [{ type: "code", code: "x" }, text(para("deep"))] },
        ],
      },
    };
    chat.setItems([item]);
    const zero = itemEl(page, "t").querySelector(".suite-text");
    chat.applyPatch(
      patch(1, {
        op: "blocks",
        ref: "t",
        path: [1, 1],
        from: 0,
        blocks: [para("deeper"), para("x")],
      }),
    );
    const texts = Array.from(itemEl(page, "t").querySelectorAll(".suite-text"));
    expect(texts).toHaveLength(2);
    expect(texts[0] === zero).toBe(true);
    expect(texts[0]?.textContent).toBe("zero");
    expect(Array.from(texts[1]?.children ?? []).map((p) => p.textContent)).toEqual(["deeper", "x"]);
    expect(itemEl(page, "t").querySelector(".suite-card-title")?.textContent).toBe("Card");

    // The card's title is not a child component: path [0] is still the first text.
    chat.applyPatch(
      patch(2, { op: "blocks", ref: "t", path: [0], from: 1, blocks: [para("more")] }),
    );
    expect(itemEl(page, "t").querySelector(".suite-text")?.textContent).toBe("zeromore");
  });

  test("a blocks op the DOM can't follow falls back to redrawing the item", () => {
    const { page, chat } = rig();
    chat.setItems([agent("m", para("a"))]);
    // Past the end: the state clamps, the text component ends up with ["a", "b"].
    chat.applyPatch(patch(1, { op: "blocks", ref: "m", path: [], from: 5, blocks: [para("b")] }));
    expect(
      Array.from(itemEl(page, "m").querySelectorAll(".suite-text > p")).map((p) => p.textContent),
    ).toEqual(["a", "b"]);
  });

  test("patches for another session are ignored; a snapshot sets the session", () => {
    const { page, chat } = rig();
    chat.applySnapshot({ session: "s1", seq: 3, items: [agent("a", para("x"))] });
    chat.applyPatch({ session: "other", seq: 4, ops: [{ op: "remove", ref: "a" }] });
    chat.applyPatch(patch(2, { op: "remove", ref: "a" }));
    expect(refs(page)).toEqual(["a"]);
    chat.applyPatch(patch(4, { op: "remove", ref: "a" }));
    expect(refs(page)).toEqual([]);
  });

  test("setItems keeps the DOM of unchanged items and handles reordering and removals", () => {
    const { page, chat } = rig();
    chat.setItems([agent("a", para("1")), agent("b", para("2")), agent("c", para("3"))]);
    const a = itemEl(page, "a");
    const c = itemEl(page, "c");
    chat.setItems([agent("c", para("3")), agent("d", para("4")), agent("a", para("1"))]);
    expect(refs(page)).toEqual(["c", "d", "a"]);
    expect(itemEl(page, "a")).toBe(a);
    expect(itemEl(page, "c")).toBe(c);
    chat.setItems([]);
    expect(refs(page)).toEqual([]);
  });

  test("a replaced thought keeps its open state, a replaced item keeps focus", () => {
    const { page, chat } = rig();
    const thought = (t: string): ViewItem => ({
      ref: "th",
      kind: "thought",
      component: text(para(t)),
    });
    chat.setItems([thought("a"), withButtons("b", "x")]);
    itemEl(page, "th").setAttribute("open", "");
    itemEl(page, "b").focus();
    chat.applyPatch(
      patch(1, { op: "set", item: thought("ab") }, { op: "set", item: withButtons("b", "x") }),
    );
    expect(itemEl(page, "th").hasAttribute("open")).toBe(true);
    expect(page.window.document.activeElement?.getAttribute("data-ref")).toBe("b");
  });
});

describe("items", () => {
  test("kinds, tool states and the user header", () => {
    const { page, chat } = rig();
    const tool = (state: ViewItem["state"]): ViewItem => ({
      ref: `t-${state}`,
      kind: "tool",
      label: `tool ${state}`,
      state,
      component: { type: "code", code: "x" },
    });
    chat.setItems([
      { ref: "u", kind: "user", label: "overlay", component: text(para("hi")) },
      { ref: "n", kind: "notice", component: text(para("note")) },
      { ref: "th", kind: "thought", component: text(para("hmm")) },
      tool("pending"),
      tool("running"),
      tool("done"),
      tool("failed"),
    ]);
    expect(itemEl(page, "u").querySelector("header")?.textContent).toBe("» overlay");
    expect(itemEl(page, "th").querySelector("summary")?.textContent).toBe("… thinking");
    const icons = ["pending", "running", "done", "failed"].map((state) => [
      state,
      itemEl(page, `t-${state}`).querySelector(".suite-state")?.textContent,
    ]);
    expect(icons).toEqual([
      ["pending", "○"],
      ["running", "◐"],
      ["done", "✓"],
      ["failed", "✗"],
    ]);
    expect(itemEl(page, "t-done").querySelector("header")?.textContent).toBe("✓ tool done");
  });

  test("guest labels head agent and thought items; unlabeled ones stay bare", () => {
    const { page, chat } = rig();
    chat.setItems([
      { ref: "a", kind: "agent", component: text(para("main")) },
      { ref: "ga", kind: "agent", label: "claude", component: text(para("guest")) },
      { ref: "t", kind: "thought", component: text(para("hmm")) },
      { ref: "gt", kind: "thought", label: "codex", component: text(para("hmm")) },
    ]);
    expect(itemEl(page, "a").querySelector("header")).toBeNull();
    expect(itemEl(page, "ga").querySelector("header")?.textContent).toBe("claude");
    expect(itemEl(page, "ga").querySelector("header")?.className).toBe("suite-item-header");
    expect(itemEl(page, "t").querySelector("summary")?.textContent).toBe("… thinking");
    expect(itemEl(page, "gt").querySelector("summary")?.textContent).toBe("… codex thinking");
  });

  test("images: asset:// and paths show the fallback, data: and https: draw an <img>", () => {
    const { page, chat } = rig();
    const image = (ref: string, src: string): ViewItem => ({
      ref,
      kind: "tool",
      component: { type: "image", src, alt: "alt text", caption: "cap", width: 4, height: 3 },
    });
    chat.setItems([
      image("asset", "asset://img-1"),
      image("path", "/tmp/a.png"),
      image("data", "data:image/png;base64,AAAA"),
      image("web", "https://example.com/a.png"),
    ]);
    expect(itemEl(page, "asset").querySelector("img")).toBeNull();
    expect(itemEl(page, "asset").querySelector(".suite-image-fallback")?.textContent).toBe(
      "[image 4×3 · asset://img-1]",
    );
    expect(itemEl(page, "path").querySelector("img")).toBeNull();
    expect(itemEl(page, "data").querySelector("img")?.getAttribute("src")).toBe(
      "data:image/png;base64,AAAA",
    );
    const web = itemEl(page, "web").querySelector("img");
    expect(web?.getAttribute("alt")).toBe("alt text");
    expect(itemEl(page, "web").querySelector("figcaption")?.textContent).toBe("cap");

    web?.dispatchEvent(new page.window.Event("error") as unknown as Event);
    expect(itemEl(page, "web").querySelector("img")).toBeNull();
    expect(itemEl(page, "web").querySelector(".suite-image-fallback")).not.toBeNull();
  });

  test("resolveImage maps asset:// to a loadable URL; unsafe results are ignored", () => {
    const { page, chat } = rig({
      resolveImage: (src) => {
        if (src === "asset://img-1") return "http://127.0.0.1:1/assets/img-1?token=t";
        if (src === "asset://img-2") return "javascript:alert(1)";
        if (src === "asset://img-3") return "file:///etc/passwd";
        return undefined;
      },
    });
    const image = (ref: string, src: string): ViewItem => ({
      ref,
      kind: "tool",
      component: { type: "image", src, width: 4, height: 3 },
    });
    chat.setItems([
      image("one", "asset://img-1"),
      image("two", "asset://img-2"),
      image("three", "asset://img-3"),
      image("four", "asset://img-4"),
      image("web", "https://example.com/a.png"),
    ]);
    expect(itemEl(page, "one").querySelector("img")?.getAttribute("src")).toBe(
      "http://127.0.0.1:1/assets/img-1?token=t",
    );
    for (const ref of ["two", "three", "four"]) {
      expect(itemEl(page, ref).querySelector("img")).toBeNull();
      expect(itemEl(page, ref).querySelector(".suite-image-fallback")).not.toBeNull();
    }
    expect(itemEl(page, "web").querySelector("img")?.getAttribute("src")).toBe(
      "https://example.com/a.png",
    );
  });

  test("links to anything but http(s) and mailto are not links", () => {
    const { page, chat } = rig();
    chat.setItems([
      {
        ref: "l",
        kind: "agent",
        component: {
          type: "stack",
          children: [
            {
              type: "text",
              blocks: [
                { type: "paragraph", spans: [{ text: "bad", href: "javascript:alert(1)" }] },
              ],
            },
            { type: "link", url: "javascript:alert(2)", text: "worse" },
            { type: "link", url: "https://ok.example/" },
          ],
        },
      },
    ]);
    const anchors = Array.from(itemEl(page, "l").querySelectorAll("a"));
    expect(anchors.map((a) => a.getAttribute("href"))).toEqual(["https://ok.example/"]);
    expect(itemEl(page, "l").textContent).toContain("bad");
    expect(itemEl(page, "l").textContent).toContain("worse");
  });
});

describe("keyboard and buttons", () => {
  test("only items with buttons are focusable", () => {
    const { page, chat } = rig();
    chat.setItems([agent("a"), withButtons("b", "x")]);
    expect(itemEl(page, "a").hasAttribute("tabindex")).toBe(false);
    expect(itemEl(page, "b").getAttribute("tabindex")).toBe("0");
  });

  test("a button's key triggers it while its item is focused", () => {
    const { page, chat, actions } = rig();
    chat.setItems([withButtons("b", "o", "c"), withButtons("other", "o")]);
    const item = itemEl(page, "b");
    item.focus();
    expect(press(page, item, "c")).toBe(false); // handled: default prevented
    expect(actions).toEqual([{ ref: "b", action: "app.act1", args: { i: 1 } }]);
    press(page, item, "o");
    expect(actions.at(-1)).toEqual({ ref: "b", action: "app.act0", args: { i: 0 } });
    // Keys the item has no button for, and modified keys, do nothing.
    expect(press(page, item, "z")).toBe(true);
    press(page, item, "o", { ctrlKey: true });
    expect(actions).toHaveLength(2);
  });

  test("the key also works while a button inside the item has focus", () => {
    const { page, chat, actions } = rig();
    chat.setItems([withButtons("b", "o")]);
    const button = itemEl(page, "b").querySelector<HTMLElement>("button");
    button?.focus();
    if (button) press(page, button, "o");
    expect(actions).toHaveLength(1);
  });

  test("a key goes only to the item it is pressed in", () => {
    const { page, chat, actions } = rig();
    chat.setItems([withButtons("one", "o"), withButtons("two", "o")]);
    press(page, itemEl(page, "two"), "o");
    expect(actions.map((a) => a.ref)).toEqual(["two"]);
    // Pressed outside any item (the container itself): nothing.
    press(page, page.container, "o");
    expect(actions).toHaveLength(1);
  });

  test("buttons without a key answer to their 1-based position", () => {
    const { page, chat, actions } = rig();
    chat.setItems([withButtons("b", undefined, "z", undefined)]);
    const item = itemEl(page, "b");
    expect(Array.from(item.querySelectorAll("kbd")).map((k) => k.textContent)).toEqual([
      "1",
      "z",
      "3",
    ]);
    press(page, item, "3");
    press(page, item, "1");
    expect(actions.map((a) => a.action)).toEqual(["app.act2", "app.act0"]);
  });

  test("arrow keys move between items with buttons, in order", () => {
    const { page, chat } = rig();
    chat.setItems([
      withButtons("a", "x"),
      agent("plain"),
      withButtons("b", "x"),
      withButtons("c", "x"),
    ]);
    const active = (): string | null | undefined =>
      page.window.document.activeElement?.getAttribute("data-ref");
    itemEl(page, "a").focus();
    press(page, itemEl(page, "a"), "ArrowDown");
    expect(active()).toBe("b");
    press(page, itemEl(page, "b"), "ArrowDown");
    expect(active()).toBe("c");
    press(page, itemEl(page, "c"), "ArrowDown");
    expect(active()).toBe("c");
    press(page, itemEl(page, "c"), "ArrowUp");
    press(page, itemEl(page, "b"), "ArrowUp");
    expect(active()).toBe("a");
    // From outside any item: Down goes to the first, Up to the last.
    press(page, page.container, "ArrowUp");
    expect(active()).toBe("c");
    press(page, page.container, "ArrowDown");
    expect(active()).toBe("a");
  });

  test("clicking a button sends its action and args", () => {
    const { page, chat, actions } = rig();
    chat.setItems([withButtons("b", "o", "c")]);
    click(page, buttonOf(itemEl(page, "b"), 1));
    expect(actions).toEqual([{ ref: "b", action: "app.act1", args: { i: 1 } }]);
  });

  test("copy: writes to the clipboard and never reaches onAction", async () => {
    const { page, chat, actions, copied } = rig();
    chat.setItems([
      {
        ref: "k",
        kind: "tool",
        component: { type: "button", label: "Copy", action: "copy:asset://img-7f3", key: "c" },
      },
    ]);
    const item = itemEl(page, "k");
    click(page, buttonOf(item, 0));
    press(page, item, "c");
    await flush();
    expect(copied).toEqual(["asset://img-7f3", "asset://img-7f3"]);
    expect(actions).toEqual([]);
    expect(item.querySelector("button")?.hasAttribute("data-copied")).toBe(true);
  });

  test("a failing clipboard is reported, not thrown", async () => {
    const { page, chat, actions, errors } = rig({
      clipboard: { writeText: () => Promise.reject(new Error("denied")) },
    });
    chat.setItems([
      {
        ref: "k",
        kind: "tool",
        component: { type: "button", label: "Copy", action: "copy:x", key: "c" },
      },
    ]);
    press(page, itemEl(page, "k"), "c");
    await flush();
    expect(errors.map((e) => e.message)).toEqual(["denied"]);
    expect(actions).toEqual([]);
  });

  test("after destroy, nothing is wired any more", () => {
    const { page, chat, actions } = rig();
    chat.setItems([withButtons("b", "o")]);
    chat.destroy();
    press(page, itemEl(page, "b"), "o");
    expect(actions).toEqual([]);
  });
});

describe("theme", () => {
  test("setTheme sets the custom properties on the container", () => {
    const { page, chat } = rig();
    const style = page.container.style;
    expect(style.getPropertyValue("--suite-accent")).toBe(theme.colors.accent);
    expect(style.getPropertyValue("--suite-space-m")).toBe("8px");
    expect(style.getPropertyValue("--suite-code-size")).toBe("13px");
    expect(style.getPropertyValue("--suite-code-font")).toBe("var(--suite-font-mono)");
    expect(style.getPropertyValue("--suite-title-weight")).toBe("600");

    const light: Theme = {
      ...theme,
      name: "light",
      colors: { ...theme.colors, bg: "#ffffff", accent: "#0044cc" },
    };
    chat.setTheme(light);
    expect(style.getPropertyValue("--suite-accent")).toBe("#0044cc");
    expect(style.getPropertyValue("--suite-bg")).toBe("#ffffff");
  });

  test("every variable the stylesheet reads is one the theme sets", () => {
    const set = new Set(Object.keys(themeVariables(theme)));
    // --suite-progress is per element (the bar's width), not theme.
    const used = new Set([...chatStyles.matchAll(/var\((--suite-[a-z-]+)\)/g)].map((m) => m[1]));
    used.delete("--suite-progress");
    expect([...used].filter((name) => name !== undefined && !set.has(name))).toEqual([]);
  });

  test("the stylesheet hard-codes no colors", () => {
    expect(chatStyles).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(chatStyles).not.toMatch(/\brgba?\(|\bhsla?\(/);
  });

  test("the stylesheet is injected once per document, and can be turned off", () => {
    const page = newPage();
    mountChat(page.container, { theme });
    const second = page.window.document.createElement("div");
    page.window.document.body.append(second);
    mountChat(second as unknown as HTMLElement, { theme });
    expect(page.window.document.querySelectorAll("style[data-suite-chat-styles]")).toHaveLength(1);

    const bare = newPage();
    mountChat(bare.container, { theme, injectStyles: false });
    expect(bare.window.document.querySelectorAll("style")).toHaveLength(0);
  });
});
