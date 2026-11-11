import { expect, test } from "bun:test";
import {
  defaultZoom,
  parseZoom,
  stepZoom,
  ZOOM_MAX,
  ZOOM_MIN,
  zoomLabel,
} from "../ui/plugins/transcript/zoom.ts";

test("a browser tab starts larger than the window", () => {
  expect(defaultZoom(true)).toBeGreaterThan(defaultZoom(false));
  expect(defaultZoom(false)).toBe(1);
});

test("steps are tenths, kept within bounds without rounding errors", () => {
  expect(stepZoom(1, 1)).toBe(1.1);
  expect(stepZoom(1.15, -1)).toBe(1.05);
  expect(stepZoom(ZOOM_MAX, 1)).toBe(ZOOM_MAX);
  expect(stepZoom(ZOOM_MIN, -1)).toBe(ZOOM_MIN);
  expect(zoomLabel(1.15)).toBe("115%");
});

test("a saved size is read back, kept within bounds, and nonsense is ignored", () => {
  expect(parseZoom('{"chatZoom":1.3}')).toBe(1.3);
  expect(parseZoom('{"chatZoom":9}')).toBe(ZOOM_MAX);
  expect(parseZoom('{"chatZoom":"big"}')).toBeUndefined();
  expect(parseZoom("not json")).toBeUndefined();
});
