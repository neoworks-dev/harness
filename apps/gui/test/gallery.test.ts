import { expect, test } from "bun:test";
import type { ViewItem } from "@neoworks-dev/neo-protocol/types";
import { chatPictures, galleryAt, stepIndex } from "../ui/plugins/transcript/gallery.ts";

const items: ViewItem[] = [
  {
    ref: "u1",
    kind: "user",
    component: {
      type: "stack",
      children: [
        { type: "text", blocks: [] },
        { type: "image", src: "/tmp/a.png" },
      ],
    },
  },
  { ref: "m-1", kind: "agent", component: { type: "text", blocks: [] } },
  {
    ref: "c-1",
    kind: "tool",
    label: "read b.png",
    component: { type: "image", src: "asset://b", alt: "the chart", caption: "Q3" },
  },
];
const resolve = (src: string) => src.replace("asset://", "http://d/asset/");

test("the chat's pictures in order, sources resolved, named by alt or file", () => {
  expect(chatPictures(items, resolve)).toEqual([
    { src: "/tmp/a.png", name: "a.png" },
    { src: "http://d/asset/b", name: "the chart", caption: "Q3" },
  ]);
});

test("a click starts the gallery at that picture; one not in the chat is shown alone", () => {
  const pictures = chatPictures(items, resolve);
  expect(galleryAt(pictures, { src: "http://d/asset/b", name: "x" }).index).toBe(1);
  expect(galleryAt(pictures, { src: "http://web/c.png", name: "c" })).toEqual({
    pictures: [{ src: "http://web/c.png", name: "c" }],
    index: 0,
  });
});

test("stepping goes round at both ends", () => {
  expect(stepIndex(2, 3, 1)).toBe(0);
  expect(stepIndex(0, 3, -1)).toBe(2);
  expect(stepIndex(0, 0, 1)).toBe(0);
});
