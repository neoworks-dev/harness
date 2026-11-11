import { expect, test } from "bun:test";
import { accept, atCursor, names } from "../ui/lib/mention.ts";

test("the word at the cursor is a mention when it starts with an @", () => {
  expect(atCursor("look at @src/ap", 15)).toEqual({ start: 8, query: "src/ap", cd: false });
  expect(atCursor("@", 1)?.query).toBe("");
  expect(atCursor("a\n@..", 5)).toEqual({ start: 2, query: "..", cd: false });
  expect(atCursor("mail me@x.de", 12)).toBeUndefined();
  expect(atCursor("@a done", 7)).toBeUndefined();
  expect(atCursor("@a@b", 4)).toBeUndefined();
});

test("a /cd argument completes to folders and names one once it ends in one", () => {
  const cd = atCursor("/cd my docs/sr", 14);
  expect(cd).toEqual({ start: 4, query: "my docs/sr", cd: true });
  expect(names(cd!)).toBe(false);
  for (const done of ["/cd ", "/cd src/", "/cd ..", "/cd ~"]) expect(names(atCursor(done, done.length)!)).toBe(true);
  expect(names(atCursor("@src/", 5)!)).toBe(false);
  expect(atCursor("/cdx", 4)).toBeUndefined();
});

test("a pick takes the word's place: a file ends it, a folder keeps it open, the rest stays", () => {
  const text = "see @ap and more";
  const mention = atCursor(text, 7)!;
  expect(accept(text, 7, mention, { path: "src/app.rs", dir: false })).toEqual({
    text: "see @src/app.rs  and more",
    cursor: 16,
  });
  expect(accept(text, 7, mention, { path: "src/", dir: true })).toEqual({
    text: "see @src/ and more",
    cursor: 9,
  });
  const cd = atCursor("/cd sr", 6)!;
  expect(accept("/cd sr", 6, cd, { path: "src/", dir: true })).toEqual({ text: "/cd src/", cursor: 8 });
});
