import { expect, test } from "bun:test";
import type { UiNode } from "@neoworks-dev/neo-protocol/types";
import { fileMark, place, stateLook, treeRows } from "../ui/plugins/transcript/neoui/dev.ts";
import { paletteOf } from "../ui/lib/palette.ts";

const file = (path: string, status?: string): UiNode => ({
  component: "File",
  props: status === undefined ? { path } : { path, status },
});

test("a file tree has folders first, joins a folder holding only a folder, and sorts by name", () => {
  const tree: UiNode = {
    component: "FileTree",
    props: {
      files: [
        file("notes.md", "untracked"),
        file("apps/term/src/menu.rs"),
        file("apps/term/src/app.rs"),
        file("apps/term/README.md"),
      ],
    },
  };
  expect(treeRows(tree).map((r) => `${"  ".repeat(r.depth)}${r.name}`)).toEqual([
    "apps/term/",
    "  src/",
    "    app.rs",
    "    menu.rs",
    "  README.md",
    "notes.md",
  ]);
});

test("statuses and states have their marks and colours", () => {
  const p = paletteOf();
  expect(fileMark("added", p)).toEqual({ mark: "A", color: p.success });
  expect(fileMark(undefined, p)).toEqual({ mark: "M", color: p.accent });
  const pr = (state: string): UiNode => ({ component: "PullRequest", props: { state } });
  expect(stateLook(pr("merged"), p).label).toBe("Merged");
  expect(stateLook(pr("closed"), p).color).toBe(p.danger);
  expect(stateLook({ component: "Issue", props: { state: "closed" } }, p).color).toBe(p.accent);
  expect(place({ component: "CodeRef", props: { path: "a.rs", line: 12 } })).toBe("a.rs:12");
});
