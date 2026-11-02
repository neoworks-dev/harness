import { expect, test } from "bun:test";
import { check, COMPONENTS } from "@neoworks/neoui";
import { UI_BRIEF } from "@neoworks-dev/neo/ui-brief";
import { GUIDANCE, uiLibraryText, uiLookupText } from "../src/gateway/ui-library.ts";

test("the neoui the brief and the guidance write evaluates without errors", () => {
  // Elements written inline: `<Progress value={0.72} label="used"/>`.
  const inline = [...`${UI_BRIEF}\n${GUIDANCE}`.matchAll(/`(<[A-Z][^`]*\/>)`/g)].map(
    (m) => m[1] ?? "",
  );
  expect(inline.length).toBeGreaterThan(0);
  for (const element of inline) {
    const { doc, repairs } = check(`<Card>${element}</Card>`);
    expect(doc.errors).toBeUndefined();
    expect(repairs).toEqual([]);
  }
  // Nothing in OpenUI, the language before neoui.
  expect(GUIDANCE).not.toContain("```openui");
});

test("the brief names every component and stays short; the reference is on demand", () => {
  for (const c of COMPONENTS.filter((c) => !c.virtual)) expect(UI_BRIEF).toContain(c.name);
  expect(UI_BRIEF.length / 3.6).toBeLessThan(450);
  expect(GUIDANCE.length / 3.6).toBeLessThan(450);
});

test("a lookup gives the components asked for, an open app's among them", () => {
  const body = { description: "A note", params: { text: "string" }, neoui: "<Text>{text}</Text>" };
  const all = { notes: { NoteCard: body } };
  const text = uiLookupText(all, new Set(["notes"]), ["TestRun", "notes.NoteCard"]);
  expect(text).toContain("<TestRun name?=");
  expect(text).toContain('<notes.NoteCard text="string"/> — A note');
  expect(text).not.toContain("# neoui");
  // A closed app's are not offered.
  expect(uiLookupText(all, new Set(), ["notes.NoteCard"])).toStartWith("Not in the library");
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
