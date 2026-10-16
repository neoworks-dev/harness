import { expect, test } from "bun:test";
import theme from "../../../protocol/themes/default.json";
import { blend, DEFAULT_THEME, paletteOf } from "../ui/lib/palette.ts";

test("the built-in theme is protocol/themes/default.json", () => {
  expect(DEFAULT_THEME).toEqual(theme);
});

test("the band mixes a little of the text colour into the background", () => {
  expect(blend("#000000", "#ffffff", 50)).toBe("#808080");
  expect(blend("#16181d", "#e6e6e6", 14)).toBe(paletteOf().band);
  expect(blend("red", "#ffffff", 50)).toBe("red");
});
