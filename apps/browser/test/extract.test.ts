import { expect, test } from "bun:test";
import { bestSrcset, decodeEntities, extractPage } from "../src/extract.ts";
import { PNG, startFixture } from "./fixture.ts";

test("decodeEntities and bestSrcset", () => {
  expect(decodeEntities("a &amp; b &lt;c&gt; &#65;&#x42; &nbsp;&bogus;")).toBe(
    "a & b <c> AB  &bogus;",
  );
  expect(bestSrcset("a.png 1x, b.png 2x")).toBe("b.png");
  expect(bestSrcset("a.png 480w, /b.png 1600w, c.png 800w")).toBe("/b.png");
  expect(bestSrcset("only.png")).toBe("only.png");
  expect(bestSrcset("")).toBeUndefined();
});

test("extracts title, readable text, links and images from the fixture page", async () => {
  const fixture = startFixture();
  try {
    const html = await (await fetch(`${fixture.url}/`)).text();
    const page = await extractPage(html, `${fixture.url}/`);
    expect(page.title).toBe("Fixture & Co");
    expect(page.text).toBe(
      "Hello page\n\nFirst paragraph with bold and a link.\n\nSecond paragraph <escaped>.\nmail other",
    );
    expect(page.text).not.toContain("secret");
    expect(page.text).not.toContain("color: red");
    expect(page.text).not.toContain("enable javascript");

    const o = fixture.url;
    // og:image first, then the page's images (best srcset candidate before src), icons last;
    // the duplicate, the tracking pixel and the data: URL are gone.
    expect(page.images).toEqual([
      { src: `${o}/og.png`, alt: "og:image" },
      { src: `${o}/logo.png`, alt: "The logo" },
      { src: `${o}/large.png`, alt: "Responsive" },
      { src: `${o}/rel/pic.png`, alt: "Responsive" },
      { src: `${o}/favicon.png`, alt: "icon" },
    ]);
    expect(page.links).toEqual([
      { href: `${o}/next`, text: "link" },
      { href: `${o}/other`, text: "other" },
    ]);
  } finally {
    await fixture.stop();
  }
  expect(PNG.length).toBeGreaterThan(8);
});

test("honours <base href> and falls back to og:title", async () => {
  const page = await extractPage(
    '<meta property="og:title" content="From meta"><base href="/sub/"><img src="a.png"><a href="b">b</a>',
    "https://example.com/x/y",
  );
  expect(page.title).toBe("From meta");
  expect(page.images).toEqual([{ src: "https://example.com/sub/a.png", alt: "" }]);
  expect(page.links).toEqual([{ href: "https://example.com/sub/b", text: "b" }]);
});

test("copes with broken markup", async () => {
  const page = await extractPage("<p>one<p>two<div><b>three", "http://x.test/");
  expect(page.text).toBe("one\ntwo\nthree");
  expect(page.title).toBe("");
  expect((await extractPage("", "http://x.test/")).text).toBe("");
});
