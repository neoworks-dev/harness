import { expect, test } from "bun:test";
import { Follow } from "../ui/widgets/follow.ts";

test("a list follows from the start, even short of the end", () => {
  const follow = new Follow();
  // Opened 173px short of the end because its rows came out taller than guessed.
  follow.scrolled(15295, 15468, 0);
  expect(follow.following).toBe(true);
  // It grows under the window: still following.
  follow.scrolled(15295, 16800, 10);
  expect(follow.following).toBe(true);
});

test("the user scrolling up lets go, reaching the end takes hold again", () => {
  const follow = new Follow();
  follow.scrolled(1000, 1000, 0);
  follow.touched(100, -120);
  expect(follow.following).toBe(false);
  follow.scrolled(700, 1200, 150);
  expect(follow.following).toBe(false);
  follow.scrolled(1198, 1200, 5000);
  expect(follow.following).toBe(true);
});

test("dragging the scrollbar up lets go: the list moves up, the content as tall as before", () => {
  const follow = new Follow();
  follow.scrolled(1000, 1000, 0);
  follow.scrolled(600, 1000, 5000);
  expect(follow.following).toBe(false);
});

test("a list that moves up without the user (its rows reflowed) still follows", () => {
  const follow = new Follow();
  follow.scrolled(1000, 1000, 0);
  // A panel opened beside it: the rows wrap anew and the window moves up, short of the end.
  follow.scrolled(800, 1400, 50);
  expect(follow.following).toBe(true);
});

test("content that shrinks pulls the window up to the end, which still follows", () => {
  const follow = new Follow();
  follow.scrolled(1000, 1000, 0);
  follow.touched(10);
  follow.scrolled(900, 900, 20);
  expect(follow.following).toBe(true);
});

test("reset follows again", () => {
  const follow = new Follow();
  follow.touched(0, -50);
  follow.reset();
  expect(follow.following).toBe(true);
});
