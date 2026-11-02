import { describe, expect, test } from "bun:test";
import type { SessionEventRecord } from "@neoworks-dev/neo-protocol/types";
import { SessionStore } from "../src/store.ts";
import { lintNote, lintUi, mistakesIn, mistakesNote, UiLint } from "../src/ui-lint.ts";

const block = (markup: string): string => `Here it is:\n\n\`\`\`neoui\n${markup}\n\`\`\`\n\nDone.`;

describe("lintUi", () => {
  test("short UI passes", () => {
    const text = block(`<Card title="Disk usage">
  <EntityList>
    <ListItem title="Sessions" subtitle="Saved by the daemon, nothing lost"><Tag tone="success">kept</Tag></ListItem>
  </EntityList>
  <Callout title="Note">Deleting the cache is safe.</Callout>
</Card>`);
    expect(lintUi(text)).toEqual([]);
  });

  test("a long subtitle, title and callout are found, the furthest over first", () => {
    const text =
      block(`<Card title="Across a daemon restart, what is kept and what is dropped for good">
  <EntityList>
    <ListItem title="Open terms" subtitle="Reconnect on their own: the status line says disconnected, reconnecting, then they pick up where they left off"/>
  </EntityList>
  <Callout tone="danger">${"word ".repeat(50)}</Callout>
</Card>`);
    const found = lintUi(text);
    expect(found.map((f) => [f.component, f.part, f.words])).toEqual([
      ["Card", "title", 13],
      ["ListItem", "subtitle", 18],
      ["Callout", "text", 50],
    ]);
  });

  test("only what is in a neoui block counts, also one still open", () => {
    const long = "one two three four five six seven eight nine ten eleven twelve thirteen";
    expect(lintUi(`<ListItem subtitle="${long}"/>`)).toEqual([]);
    expect(lintUi(`\`\`\`neoui\n<ListItem subtitle="${long}"/>`)).toHaveLength(1);
  });

  test("a value with > in it doesn't end the tag", () => {
    const text = block(
      `<Metric value={a > b ? 1 : 2} label="one two three four five six seven eight nine"/>`,
    );
    expect(lintUi(text).map((f) => f.part)).toEqual(["label"]);
  });

  test("the note names at most three and how long each may be", () => {
    const found = lintUi(
      block(
        Array.from(
          { length: 5 },
          (_, i) => `<ListItem title="t" subtitle="${"w ".repeat(14 + i)}x${i}"/>`,
        ).join("\n"),
      ),
    );
    const note = lintNote(found);
    expect(note).toContain("a ListItem's subtitle");
    expect(note).toContain("keep it under 12");
    expect(note.match(/^- /gm)).toHaveLength(4);
    expect(note).toContain("and 2 more");
  });
});

describe("mistakes", () => {
  test("a wrong attribute or a missing one is told with what the component takes", () => {
    const wrong = mistakesIn(block('<TestRun name="x" pased={3}/>\n<PullRequest title="y"/>'));
    expect(wrong).toEqual([
      { message: "TestRun.pased is not an attribute", component: "TestRun" },
      { message: "PullRequest.number is required", component: "PullRequest" },
    ]);
    const note = mistakesNote(wrong);
    expect(note).toStartWith("Your last reply's UI had mistakes");
    expect(note).toContain(
      '- TestRun.pased is not an attribute. It takes: <TestRun name?="string" passed?={number}',
    );
    expect(note).toContain(
      "- PullRequest.number is required. It takes: <PullRequest number={number}",
    );
  });

  test("an unknown component gets the nearest name; an app's own is known", () => {
    const note = mistakesNote(mistakesIn(block('<Card><Metrc label="a" value={1}/></Card>')));
    expect(note).toContain("- Unknown component Metrc; did you mean Metric?");
    const apps = {
      photo: {
        Crop: {
          description: "A crop box",
          params: { asset: "string" },
          neoui: "<Text>{asset}</Text>",
        },
      },
    };
    expect(mistakesIn(block('<Card><photo.Crop asset="a"/></Card>'), apps)).toEqual([]);
    expect(mistakesIn(block('<Card title="fine"><Metric label="a" value={1}/></Card>'))).toEqual(
      [],
    );
  });
});

describe("UiLint", () => {
  test("a turn whose UI ran long leaves the agent a note; a guest's text doesn't count", async () => {
    const store = new SessionStore(":memory:");
    store.createSession({ id: "s", cwd: "/" });
    new UiLint(store);
    const long = `<ListItem title="t" subtitle="${"w ".repeat(20)}"/>`;
    store.append("s", { type: "user_message", app: "cli", text: "hi" });
    store.append("s", { type: "agent_message_chunk", messageId: "m1", text: "```neo" });
    store.append("s", { type: "agent_message_chunk", messageId: "m1", text: `ui\n${long}\n` });
    store.append("s", { type: "agent_message_chunk", messageId: "m1", text: "```" });
    store.append("s", { type: "turn_ended", stopReason: "end_turn" });
    await Bun.sleep(0);
    const notes = (): SessionEventRecord[] =>
      store.read("s").filter((r) => r.event.type === "user_note");
    expect(notes()).toHaveLength(1);
    expect(notes()[0]?.event).toMatchObject({ type: "user_note", about: "neoui" });

    store.append("s", { type: "user_message", app: "cli", text: "again" });
    store.append("s", {
      type: "agent_message_chunk",
      messageId: "m2",
      agent: "claude",
      text: block(long),
    });
    store.append("s", { type: "turn_ended", stopReason: "end_turn" });
    await Bun.sleep(0);
    expect(notes()).toHaveLength(1);
  });
});
