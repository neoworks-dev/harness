import { expect, test } from "bun:test";
import theme from "../../../protocol/themes/default.json";
import { blend, DEFAULT_THEME, paletteOf } from "../ui/lib/palette.ts";

test("the built-in theme is protocol/themes/default.json", () => {
  expect(DEFAULT_THEME).toEqual(theme);
});

test("blend mixes one colour into another", () => {
  expect(blend("#000000", "#ffffff", 50)).toBe("#808080");
  expect(blend("red", "#ffffff", 50)).toBe("red");
});

test("the window keeps its own greys, and its accent is their white, not the theme's hue", () => {
  const custom = {
    ...DEFAULT_THEME,
    colors: { ...DEFAULT_THEME.colors, bg: "#ffffff", accent: "#ff0000" },
  };
  expect(paletteOf(custom).bg).toBe(paletteOf().bg);
  expect(paletteOf(custom).accent).toBe(paletteOf().fg);
  // Code still takes the theme's hue for its functions.
  expect(paletteOf(custom).syntax.function).toBe("#ff0000");
});
