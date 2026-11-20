import { expect, test } from "bun:test";
import type { SurfacePlace, ViewItem } from "@neoworks-dev/neo-protocol/types";
import { inPlace, pageAfter, titleOf } from "../ui/plugins/surfaces/surfaces.ts";

const surface = (name: string, place: SurfacePlace, label?: string): ViewItem => ({
  ref: `s-${name}`,
  kind: "tool",
  place,
  ...(label ? { label } : {}),
  component: { type: "neoui", ui: { id: "0" } },
});

test("surfaces are sorted by place, in the order they opened", () => {
  const all = [surface("a", "bottom"), surface("b", "side"), surface("c", "bottom")];
  expect(inPlace(all, "bottom").map((s) => s.ref)).toEqual(["s-a", "s-c"]);
  expect(inPlace(all, "page")).toEqual([]);
});

test("a page that opens is shown; the shown one stays while it is open; closed, the chat", () => {
  const one = [surface("vars", "page")];
  expect(pageAfter(undefined, one, new Set())).toBe("s-vars");
  // Back to the chat: an old page does not come back by itself.
  expect(pageAfter(undefined, one, new Set(["s-vars"]))).toBeUndefined();
  const two = [...one, surface("chart", "page")];
  expect(pageAfter("s-vars", two, new Set(["s-vars"]))).toBe("s-chart");
  expect(pageAfter("s-vars", two, new Set(["s-vars", "s-chart"]))).toBe("s-vars");
  expect(
    pageAfter("s-vars", [surface("chart", "page")], new Set(["s-vars", "s-chart"])),
  ).toBeUndefined();
});

test("a surface is called by its title, else its name", () => {
  expect(titleOf(surface("train", "bottom", "Training"))).toBe("Training");
  expect(titleOf(surface("train", "bottom"))).toBe("train");
});
