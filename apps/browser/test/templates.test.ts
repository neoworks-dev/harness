// The templates against sample tool results, resolved by the daemon's own resolver (which also
// validates the result against the view schema and throws if it is not valid).
import { describe, expect, test } from "bun:test";
import type { Component } from "@neoworks-dev/neo-protocol/types";
import { resolveTemplate } from "../../../daemon/src/view/template.ts";
import { browserTemplates } from "../src/tools.ts";

function template(name: string): unknown {
  return browserTemplates[name];
}

function find(component: Component, type: string): Component[] {
  const found: Component[] = [];
  if (component.type === type) found.push(component);
  if ("children" in component) for (const c of component.children) found.push(...find(c, type));
  return found;
}

const image = (n: number): { src: string; alt: string } => ({
  src: `https://example.com/${n}.png`,
  alt: `image ${n}`,
});

describe("navigate", () => {
  const data = (images: { src: string; alt: string }[]): Record<string, unknown> => ({
    url: "https://example.com/",
    title: "Example",
    text: "Some text",
    excerpt: "Some text",
    images,
    imageCount: images.length,
    links: 4,
  });

  test("a card with a link, counts, and thumbnails with keyed download buttons", () => {
    const card = resolveTemplate(template("navigate"), data([1, 2, 3, 4].map(image)));
    expect(card).toMatchObject({ type: "card", title: "Example" });
    expect(find(card, "link")).toEqual([
      { type: "link", url: "https://example.com/", text: "https://example.com/" },
    ]);
    expect(find(card, "image")).toHaveLength(3);
    expect(find(card, "button")).toEqual([
      {
        type: "button",
        label: "Download 1",
        action: "browser.download",
        args: { url: "https://example.com/1.png" },
        key: "1",
      },
      expect.objectContaining({ key: "2", args: { url: "https://example.com/2.png" } }),
      expect.objectContaining({ key: "3", args: { url: "https://example.com/3.png" } }),
    ]);
  });

  test("fewer than three images, or none, leave out the missing thumbnails", () => {
    expect(find(resolveTemplate(template("navigate"), data([image(1)])), "button")).toHaveLength(1);
    const none = resolveTemplate(template("navigate"), data([]));
    expect(find(none, "image")).toHaveLength(0);
    expect(find(none, "button")).toHaveLength(0);
    expect(find(none, "row")).toHaveLength(0);
  });
});

describe("download", () => {
  const base = {
    handle: "asset://img-0123456789ab",
    name: "logo.png",
    mime: "image/png",
    size: 1234,
    url: "https://example.com/logo.png",
    path: "/data/assets/img-0123456789ab.png",
  };

  test("an image: the asset handle is the image src, with copy buttons", () => {
    const card = resolveTemplate(template("download"), { ...base, isImage: true, isOther: false });
    expect(card).toMatchObject({ type: "card", title: "Downloaded logo.png" });
    expect(find(card, "image")).toEqual([
      {
        type: "image",
        src: "asset://img-0123456789ab",
        alt: "logo.png",
        caption: "image/png · 1234 bytes",
      },
    ]);
    expect(find(card, "file")).toHaveLength(0);
    expect(find(card, "button")).toEqual([
      { type: "button", label: "Copy handle", action: "copy:asset://img-0123456789ab", key: "c" },
      {
        type: "button",
        label: "Copy source URL",
        action: "copy:https://example.com/logo.png",
        key: "u",
      },
    ]);
    expect(find(card, "kv")[0]).toMatchObject({
      items: [
        { key: "Handle", value: "asset://img-0123456789ab" },
        { key: "Type", value: "image/png" },
        { key: "From", value: "https://example.com/logo.png" },
      ],
    });
  });

  test("anything else: a file chip instead of the image", () => {
    const card = resolveTemplate(template("download"), {
      ...base,
      name: "a.zip",
      mime: "application/zip",
      isImage: false,
      isOther: true,
    });
    expect(find(card, "image")).toHaveLength(0);
    expect(find(card, "file")).toEqual([
      { type: "file", path: "/data/assets/img-0123456789ab.png", name: "a.zip" },
    ]);
  });
});

describe("read and images", () => {
  test("read is a one-line note", () => {
    const text = resolveTemplate(template("read"), { title: "T", chars: 42 });
    expect(JSON.stringify(text)).toContain("42 characters");
  });

  test("images is a table of alt and url", () => {
    const card = resolveTemplate(template("images"), {
      title: "T",
      images: [image(1), image(2)],
      count: 2,
    });
    expect(card).toMatchObject({ type: "card", title: "Images on T" });
    expect(find(card, "table")).toEqual([
      {
        type: "table",
        columns: ["alt", "src"],
        rows: [
          ["image 1", "https://example.com/1.png"],
          ["image 2", "https://example.com/2.png"],
        ],
      },
    ]);
  });
});
