import { expect, test } from "bun:test";
import { Follow } from "../ui/widgets/follow.ts";

test("a list follows from the start, even short of the end", () => {
  const follow = new Follow();
  // Opened 173px short of the end because its rows came out taller than guessed.
  follow.scrolled(15295, 15468);
  expect(follow.following).toBe(true);
  // It grows under the window: still following.
  follow.scrolled(15295, 16800);
  expect(follow.following).toBe(true);
});

test("scrolling up lets go, reaching the end takes hold again", () => {
  const follow = new Follow();
  follow.scrolled(1000, 1000);
  follow.scrolled(600, 1000);
  expect(follow.following).toBe(false);
  follow.scrolled(700, 1200);
  expect(follow.following).toBe(false);
  follow.scrolled(1198, 1200);
  expect(follow.following).toBe(true);
});

test("content that shrinks pulls the window up to the end, which still follows", () => {
  const follow = new Follow();
  follow.scrolled(1000, 1000);
  follow.scrolled(900, 900);
  expect(follow.following).toBe(true);
});

test("reset follows again", () => {
  const follow = new Follow();
  follow.scrolled(1000, 1000);
  follow.scrolled(100, 1000);
  follow.reset();
  expect(follow.following).toBe(true);
});
