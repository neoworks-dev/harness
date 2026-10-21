import { expect, test } from "bun:test";
import { check, COMPONENTS } from "@neoworks/neoui";
import { GUIDANCE, uiLibraryText } from "../src/gateway/ui-library.ts";

test("the guidance's examples are neoui, and evaluate without errors", () => {
  const programs = [...GUIDANCE.matchAll(/```neoui\n([\s\S]*?)\n```/g)].map((m) => m[1] ?? "");
  expect(programs.length).toBeGreaterThan(0);
  for (const program of programs) {
    const { doc, repairs } = check(program);
    expect(doc.errors).toBeUndefined();
    expect(repairs).toEqual([]);
  }
  expect(GUIDANCE).not.toContain("```openui");
});

test("every component the guidance names is in the library", () => {
  const known = new Set(COMPONENTS.map((c) => c.name));
  // Tags written in the guidance: `<Name …`, but not the placeholder app component or the
  // connect announcement.
  const named = [...GUIDANCE.matchAll(/<([A-Z][A-Za-z]*)\b/g)].map((m) => m[1] ?? "");
  expect(named.filter((name) => !known.has(name))).toEqual([]);
  // Words in backticks that are components: Grid, FollowUpBlock, EntityList…
  const ticked = [...GUIDANCE.matchAll(/`([A-Z][A-Za-z]+)`/g)].map((m) => m[1] ?? "");
  expect(ticked.filter((name) => !known.has(name))).toEqual([]);
});

test("only the open apps' components are listed", () => {
  const body = { description: "A note", params: { text: "string" }, neoui: "<Text>{text}</Text>" };
  const all = { notes: { NoteCard: body }, photo: { PhotoCard: body } };
  const text = uiLibraryText(all, new Set(["photo"]));
  expect(text).toContain('<photo.PhotoCard text="string"/> — A note');
  expect(text).not.toContain("NoteCard");
  expect(uiLibraryText(all, new Set())).toContain("No connected app adds components right now.");
});
