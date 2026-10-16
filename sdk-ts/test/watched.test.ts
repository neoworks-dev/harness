import { describe, expect, test } from "bun:test";
import { TimeoutError } from "../src/errors.ts";
import { WatchedValue } from "../src/watched.ts";

function watch<T>(initial: T, onError: (error: Error) => void = () => {}): WatchedValue<T> {
  return new WatchedValue<T>("the thing", initial, onError);
}

describe("value and on", () => {
  test("value is the latest set; on hears each new value, not the current one", () => {
    const watched = watch<number | undefined>(undefined);
    const heard: (number | undefined)[] = [];
    watched.on((value) => heard.push(value));
    expect(watched.value).toBeUndefined();
    watched.set(1);
    watched.set(1);
    watched.set(2);
    expect(watched.value).toBe(2);
    expect(heard).toEqual([1, 1, 2]);
  });

  test("the function on returns stops listening, and may be called twice", () => {
    const watched = watch(0);
    const heard: number[] = [];
    const stop = watched.on((value) => heard.push(value));
    watched.set(1);
    stop();
    stop();
    watched.set(2);
    expect(heard).toEqual([1]);
  });

  test("a callback may stop itself while being called", () => {
    const watched = watch(0);
    const heard: string[] = [];
    const stop = watched.on(() => {
      heard.push("first");
      stop();
    });
    watched.on(() => heard.push("second"));
    watched.set(1);
    watched.set(2);
    expect(heard).toEqual(["first", "second", "second"]);
  });

  test("a throwing callback goes to onError and does not stop the others", () => {
    const errors: Error[] = [];
    const watched = watch(0, (error) => errors.push(error));
    const heard: number[] = [];
    watched.on(() => {
      throw new Error("bad listener");
    });
    watched.on((value) => heard.push(value));
    watched.set(7);
    expect(heard).toEqual([7]);
    expect(errors.map((error) => error.message)).toEqual(["bad listener"]);
  });
});

describe("waitFor", () => {
  test("resolves at once when a value is already there", async () => {
    expect(await watch("now").waitFor(50)).toBe("now");
  });

  test("waits for the first value that is not undefined", async () => {
    const watched = watch<string | undefined>(undefined);
    const waiting = watched.waitFor(1000);
    watched.set(undefined);
    watched.set("later");
    expect(await waiting).toBe("later");
  });

  test("with accept, waits for the first value it approves", async () => {
    const watched = watch(1);
    const waiting = watched.waitFor(1000, (value) => value >= 3);
    watched.set(2);
    watched.set(3);
    watched.set(4);
    expect(await waiting).toBe(3);
  });

  test("rejects with TimeoutError, naming what it waited for, and stops listening", async () => {
    const watched = watch<number | undefined>(undefined);
    const error = await watched.waitFor(30).catch((thrown: unknown) => thrown);
    expect(error).toBeInstanceOf(TimeoutError);
    expect(String(error)).toContain("30 ms waiting for the thing");
    // The finished wait left nothing behind that a later value could trip over.
    watched.set(1);
    expect(watched.value).toBe(1);
  });
});
