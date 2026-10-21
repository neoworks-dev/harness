// The browser's MCP tools and the templates its results are drawn with.
import type { ToolResult, ToolSpec } from "@neoworks-dev/neo-sdk";
import { NO_PAGE, type Browser, type LoadedPage } from "./browser.ts";
import { truncate } from "./extract.ts";

const NAVIGATE_TEXT = 4000;
const READ_TEXT = 50_000;
const NAVIGATE_IMAGES = 20;
const EXCERPT = 240;

const imageSchema = {
  type: "object",
  properties: { src: { type: "string" }, alt: { type: "string" } },
  required: ["src", "alt"],
};

function stringArg(args: Record<string, unknown>, name: string): string {
  const value = args[name];
  if (typeof value !== "string" || value.trim() === "") throw new Error(`${name} is required`);
  return value;
}

/** `url (alt)`, or just the url. */
function describe(image: { src: string; alt: string }): string {
  if (image.alt === "") return image.src;
  return `${image.src} (${image.alt})`;
}

function summary(page: LoadedPage): string {
  const lines = [`${page.title === "" ? "(untitled)" : page.title}`, page.url, ""];
  lines.push(truncate(page.text, NAVIGATE_TEXT) || "(no text)");
  if (page.images.length > 0) {
    lines.push("", `Images (${page.images.length}):`);
    for (const image of page.images.slice(0, 10)) {
      lines.push(`- ${describe(image)}`);
    }
    if (page.images.length > 10) lines.push("- … call images for the rest");
  }
  return lines.join("\n");
}

export function browserTools(browser: Browser): ToolSpec[] {
  return [
    {
      name: "navigate",
      description:
        "Open a web page (http/https; a bare host like example.com gets https). Shows it in the browser and returns its title, readable text, images and link count.",
      inputSchema: {
        type: "object",
        properties: {
          url: { type: "string", description: "Absolute URL, or relative to the current page" },
        },
        required: ["url"],
      },
      outputSchema: {
        type: "object",
        properties: {
          url: { type: "string" },
          title: { type: "string" },
          text: { type: "string", description: "Readable text, cut short; call read for more" },
          excerpt: { type: "string" },
          images: { type: "array", items: imageSchema },
          imageCount: { type: "integer" },
          links: { type: "integer", description: "How many links the page has" },
        },
        required: ["url", "title", "text", "images", "links"],
      },
      async handler(args): Promise<ToolResult> {
        const page = await browser.navigate(stringArg(args, "url"));
        return {
          content: [{ type: "text", text: summary(page) }],
          structuredContent: {
            url: page.url,
            title: page.title,
            text: truncate(page.text, NAVIGATE_TEXT),
            excerpt: truncate(page.text.replace(/\s+/g, " "), EXCERPT),
            images: page.images.slice(0, NAVIGATE_IMAGES),
            imageCount: page.images.length,
            links: page.links.length,
          },
        };
      },
    },
    {
      name: "read",
      description: "The current page's readable text (up to about 50,000 characters).",
      inputSchema: { type: "object", properties: {} },
      outputSchema: {
        type: "object",
        properties: {
          url: { type: "string" },
          title: { type: "string" },
          text: { type: "string" },
          chars: { type: "integer" },
        },
        required: ["url", "title", "text", "chars"],
      },
      handler(): Promise<ToolResult> {
        const { page } = browser;
        if (!page) return Promise.reject(new Error(NO_PAGE));
        const text = truncate(page.text, READ_TEXT);
        return Promise.resolve({
          content: [{ type: "text", text: text === "" ? "(no text)" : text }],
          structuredContent: { url: page.url, title: page.title, text, chars: text.length },
        });
      },
    },
    {
      name: "images",
      description:
        "The current page's images (absolute URLs and alt text, from <img>, srcset, og:image and icons). Pass a URL to download.",
      inputSchema: { type: "object", properties: {} },
      outputSchema: {
        type: "object",
        properties: {
          url: { type: "string" },
          title: { type: "string" },
          images: { type: "array", items: imageSchema },
          count: { type: "integer" },
        },
        required: ["url", "title", "images", "count"],
      },
      handler(): Promise<ToolResult> {
        const { page } = browser;
        if (!page) return Promise.reject(new Error(NO_PAGE));
        const lines = page.images.map((image, i) => `${i + 1}. ${describe(image)}`);
        return Promise.resolve({
          content: [{ type: "text", text: lines.join("\n") || "(no images)" }],
          structuredContent: {
            url: page.url,
            title: page.title,
            images: page.images,
            count: page.images.length,
          },
        });
      },
    },
    {
      name: "download",
      description:
        "Fetch a file (up to 10 MB) into the suite's asset store and return an asset:// handle that other apps' tools accept. The url may be relative to the current page.",
      inputSchema: {
        type: "object",
        properties: { url: { type: "string", description: "Absolute, or relative to the page" } },
        required: ["url"],
      },
      outputSchema: {
        type: "object",
        properties: {
          handle: { type: "string", description: "asset://…" },
          name: { type: "string" },
          mime: { type: "string" },
          size: { type: "integer" },
          url: { type: "string" },
          path: { type: "string" },
          isImage: { type: "boolean" },
          isOther: { type: "boolean" },
        },
        required: ["handle", "name", "mime", "size"],
      },
      async handler(args): Promise<ToolResult> {
        const download = await browser.download(stringArg(args, "url"));
        const isImage = download.mime.startsWith("image/");
        return {
          content: [
            {
              type: "text",
              text: `${download.handle} (${download.name}, ${download.mime}, ${download.size} bytes)`,
            },
          ],
          structuredContent: { ...download, isImage, isOther: !isImage },
        };
      },
    },
  ];
}

/** Keyed by tool name, as `app.presence.templates` wants (format: protocol/README.md). */
export const browserTemplates: Record<string, unknown> = {
  navigate: {
    type: "card",
    title: "{{data.title}}",
    children: [
      { type: "link", url: "{{data.url}}", text: "{{data.url}}" },
      { type: "text", markdown: "{{data.excerpt}}", tone: "muted", if: "data.excerpt" },
      {
        type: "text",
        markdown: "{{data.imageCount}} images · {{data.links}} links",
        tone: "muted",
      },
      {
        type: "row",
        gap: "m",
        if: "data.images.0",
        children: [
          {
            type: "stack",
            each: "data.images",
            as: "img",
            limit: 3,
            children: [
              { type: "image", src: "{{img.src}}", alt: "{{img.alt}}", width: 120 },
              {
                type: "button",
                label: "Download {{index}}",
                action: "browser.download",
                args: { url: "{{img.src}}" },
                key: "{{index}}",
              },
            ],
          },
        ],
      },
    ],
  },
  read: {
    type: "text",
    markdown: "Read **{{data.title}}**: {{data.chars}} characters of text.",
    tone: "muted",
  },
  images: {
    type: "card",
    title: "Images on {{data.title}}",
    children: [{ type: "table", columns: ["alt", "src"], rows: "{{data.images}}" }],
  },
  download: {
    type: "card",
    title: "Downloaded {{data.name}}",
    children: [
      {
        type: "image",
        src: "{{data.handle}}",
        alt: "{{data.name}}",
        caption: "{{data.mime}} · {{data.size}} bytes",
        if: "data.isImage",
      },
      { type: "file", path: "{{data.path}}", name: "{{data.name}}", if: "data.isOther" },
      {
        type: "kv",
        items: { Handle: "{{data.handle}}", Type: "{{data.mime}}", From: "{{data.url}}" },
      },
      {
        type: "row",
        children: [
          { type: "button", label: "Copy handle", action: "copy:{{data.handle}}", key: "c" },
          { type: "button", label: "Copy source URL", action: "copy:{{data.url}}", key: "u" },
        ],
      },
    ],
  },
};
