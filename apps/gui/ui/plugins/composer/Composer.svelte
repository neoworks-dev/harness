<script lang="ts">
  // Where the user types: a rounded card with the input over the session's pickers and the send
  // button, or, once the agent asks a question (a permission or a choice) and the user has paused
  // typing, the panel that answers it.
  import { tick, untrack } from "svelte";
  import { Button, Icon, Input, Overlay, Scroll, Text, View } from "@neoworks/svelte-gpui/ui";
  import arrowUp from "@phosphor-icons/core/bold/arrow-up-bold.svg";
  import stop from "@phosphor-icons/core/fill/stop-fill.svg";
  import chatText from "@phosphor-icons/core/regular/chat-text.svg";
  import paperclip from "@phosphor-icons/core/regular/paperclip.svg";
  import xIcon from "@phosphor-icons/core/regular/x.svg";
  import type { KeyEvent, Layout } from "@neoworks/svelte-gpui/ui";
  import { useCtx } from "../../kernel/slots/context.ts";
  import { preview, rangeLabel } from "../../lib/comments.ts";
  import Slot from "../../kernel/slots/Slot.svelte";
  import CommandMenu from "./CommandMenu.svelte";
  import Help from "./Help.svelte";
  import MentionMenu from "./MentionMenu.svelte";
  import { mentions } from "./mentions.svelte.ts";
  import Pickers from "./Pickers.svelte";
  import PromptPanel from "./PromptPanel.svelte";
  import Queue from "./Queue.svelte";
  import Thumbnail from "./Thumbnail.svelte";
  import { chatZoom } from "../transcript/zoom.svelte.ts";
  import { composer, isImage } from "./state.svelte.ts";

  const { daemon, keyboard, transcript } = useCtx();

  const p = $derived(daemon.palette);
  /** The input grows with its text up to this many pixels, then scrolls. */
  const MAX_INPUT_HEIGHT = 200;
  /** The input's text, larger than the chat's body, and as much larger again as the chat is zoomed. */
  const FONT_SIZE = 16;
  const fontSize = $derived(Math.round(FONT_SIZE * chatZoom.value));
  const LINE_HEIGHT = $derived(Math.round(fontSize * 1.5));

  let input: ReturnType<typeof Input> | undefined = $state();
  let scroller: ReturnType<typeof Scroll> | undefined = $state();
  let width = $state(0);
  /** How many characters fit a row, roughly; the input does not say where its cursor is. */
  const columns = $derived(Math.floor(width / (fontSize * 0.6)));

  const promptRef = $derived(composer.prompt?.ref);
  const showPanel = $derived(composer.engaged && composer.prompt !== undefined);

  // Another session: its own draft and history.
  $effect(() => {
    const session = daemon.sessionId;
    untrack(() => void composer.switchSession(session));
  });

  // A question takes the input's place once there is no draft and the user has paused typing.
  $effect(() => {
    const wait = composer.syncEngaged();
    if (wait === undefined) return;
    const timer = setTimeout(() => composer.syncEngaged(), wait);
    return () => clearTimeout(timer);
  });

  // A new question starts on its current button, before it is drawn.
  $effect.pre(() => {
    void promptRef;
    untrack(() => composer.startPrompt(composer.prompt));
  });

  // The chat draws a one-line stub for the question the panel is answering.
  const answeringRef = $derived(showPanel ? promptRef : undefined);
  $effect(() => {
    if (answeringRef !== undefined) return transcript.stub(answeringRef);
  });

  // The input has the keyboard unless a pane (the sidebar, the agents list, the GitHub view)
  // took it. svelte-gpui can't hand focus back yet (neoworks-dev/svelte-gpui#41), hence the claims.
  $effect(() => {
    if (!showPanel && !keyboard.taken) input?.focus();
  });

  /** The input's text and cursor as they are now, for the `@` menu. */
  function track(): void {
    const cursor = input?.getSelection()?.start ?? composer.text.length;
    mentions.track(composer.text, cursor, daemon.cwd);
  }

  // A draft changed from outside the input (history, a pick, another session) or another directory.
  $effect(() => {
    void composer.text;
    void daemon.cwd;
    untrack(track);
  });

  /** Puts the `@` menu's pick in the word's place, the cursor after it. */
  function pickMention(index?: number): void {
    const picked = mentions.pick(index);
    if (picked === undefined) return;
    composer.text = picked.text;
    input?.focus();
    void tick().then(() => {
      input?.select(picked.cursor, picked.cursor);
      mentions.track(picked.text, picked.cursor, daemon.cwd);
    });
  }

  /** Keys for the `@` menu while it is open; true when one was used. */
  function mentionKey(event: KeyEvent): boolean {
    if (!mentions.visible || event.ctrl || event.alt || event.meta) return false;
    switch (event.key) {
      case "ArrowUp":
      case "ArrowDown":
        mentions.move(event.key === "ArrowUp" ? -1 : 1);
        break;
      case "Tab":
        if (event.shift) return false;
        pickMention();
        break;
      case "Enter":
        if (event.shift || !mentions.entersPick) return false;
        pickMention();
        break;
      case "Escape":
        mentions.close();
        event.stopPropagation();
        break;
      default:
        return false;
    }
    event.preventDefault();
    return true;
  }

  function key(event: KeyEvent): void {
    if (mentionKey(event)) return;
    // The cursor moves after the key: where it went decides the `@` word.
    if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) setTimeout(track, 0);
    // Ctrl-V also attaches an image on the clipboard; the input pastes any text itself.
    if (event.ctrl && !event.alt && event.key.toLowerCase() === "v") void composer.pasteImage();
    const plain = !event.ctrl && !event.alt && !event.meta;
    switch (event.key) {
      case "Enter":
        // Shift-Enter is left to the input, which breaks the line. Alt-Enter and Ctrl-Enter send
        // into the running turn at once, as the terminal's do.
        if (event.shift) return;
        event.preventDefault();
        composer.submit(event.alt || event.ctrl);
        break;
      case "Tab":
        if (event.shift || !composer.menuVisible) return;
        event.preventDefault();
        composer.complete();
        break;
      case "ArrowUp":
      case "ArrowDown": {
        if (!plain && !event.alt) return;
        const older = event.key === "ArrowUp";
        if (composer.menuVisible) {
          composer.moveMenu(older ? -1 : 1);
        } else if (older && plain && composer.canUnqueue) {
          // Messages queued behind the running turn come back to be edited before the history.
          void composer.unqueue();
        } else if (!composer.walkHistory(older ? "older" : "newer", columns, event.alt)) {
          return;
        }
        event.preventDefault();
        break;
      }
      default:
        return;
    }
  }

  function typed(): void {
    composer.typed();
    track();
  }

  function layout(box: Layout): void {
    width = box.width;
  }

  /** Follows the end of the text while it grows past the cap. */
  function grown(box: Layout): void {
    if (box.height > MAX_INPUT_HEIGHT) scroller?.scrollTo(box.height);
  }
</script>

<View style={{ paddingTop: 8, paddingBottom: 12 }}>
  <Queue onback={() => input?.focus()} />
  <View
    style={{
      background: p.surface,
      radius: 22,
      borderWidth: 1,
      borderColor: p.border,
      shadow: { y: 1, blur: 0, color: "rgba(255, 255, 255, 0.03)" },
    }}
  >
    {#if showPanel}
      <View style={{ padding: 16 }}>
        <PromptPanel />
      </View>
    {:else}
      {#if composer.waiting && composer.prompt}
        <View
          onpress={() => composer.answerWaiting()}
          hover={{ color: p.fg }}
          style={{ direction: "row", gap: 6, paddingX: 16, paddingTop: 12, color: p.muted, cursor: "pointer" }}
        >
          <Text style={{ color: p.accent, shrink: 0 }}>{composer.prompt.title || "A question"} is waiting.</Text>
          <Text style={{ truncate: true, minWidth: 0 }}>Press here to answer it now, or send or clear your message. Esc puts it aside.</Text>
        </View>
      {/if}
      {#if composer.attachments.length > 0 || composer.comments.length > 0}
        <View style={{ direction: "row", wrap: true, gap: 6, paddingX: 14, paddingTop: 12 }}>
          {#each composer.attachments as path (path)}
            {#if isImage(path)}
              <Thumbnail {path} onremove={() => composer.detach(path)} />
            {:else}
              <View style={{ direction: "row", align: "center", gap: 4, paddingLeft: 10, paddingRight: 4, height: 26, radius: p.radius.s, background: p.raised }}>
                <Text style={{ fontSize: 12, maxWidth: 220, truncate: true }}>{path.slice(path.lastIndexOf("/") + 1)}</Text>
                <Button onpress={() => composer.detach(path)} style={{ size: 18, align: "center", justify: "center", radius: 4, color: p.muted }} hover={{ color: p.fg, background: p.hover }}>
                  <Icon svg={xIcon} size={12} />
                </Button>
              </View>
            {/if}
          {/each}
          {#each composer.comments as comment (comment.id)}
            <View style={{ direction: "row", align: "center", gap: 6, paddingLeft: 8, paddingRight: 4, height: 26, maxWidth: "100%", radius: p.radius.s, background: p.raised }}>
              <Icon svg={chatText} size={13} />
              <Text style={{ fontSize: 12, minWidth: 0, shrink: 1, truncate: true }}>
                {rangeLabel(comment)}<Text style={{ color: p.muted }}>{"  "}{preview(comment.text, 40)}</Text>
              </Text>
              <Button onpress={() => composer.uncomment(comment.id)} style={{ size: 18, align: "center", justify: "center", radius: 4, color: p.muted }} hover={{ color: p.fg, background: p.hover }}>
                <Icon svg={xIcon} size={12} />
              </Button>
            </View>
          {/each}
        </View>
      {/if}
      <View style={{ paddingX: 16, paddingTop: 16, paddingBottom: 8 }} onpress={() => input?.focus()}>
        <View style={{ minWidth: 0 }} onlayout={layout}>
          <Scroll bind:this={scroller} style={{ maxHeight: MAX_INPUT_HEIGHT }}>
            <View onlayout={grown} style={{ minHeight: LINE_HEIGHT * 2 }}>
              <Input
                bind:this={input}
                bind:value={composer.text}
                multiline
                rows={2}
                placeholder="Ask anything, or / for commands"
                style={{ color: p.fg, fontSize, lineHeight: LINE_HEIGHT }}
                oninput={typed}
                onkey={key}
              />
            </View>
          </Scroll>
          {#if composer.menuVisible}
            <Overlay side="top" align="start" offset={24} matchWidth>
              <CommandMenu onchosen={() => input?.focus()} />
            </Overlay>
          {:else if mentions.visible}
            <Overlay side="top" align="start" offset={24} matchWidth>
              <MentionMenu onpick={pickMention} />
            </Overlay>
          {/if}
          {#if composer.helpOpen}
            <Overlay side="top" align="start" offset={24} matchWidth>
              <Help />
            </Overlay>
          {/if}
        </View>
      </View>
      <View style={{ direction: "row", align: "center", gap: 8, paddingX: 10, paddingBottom: 10 }}>
        <Slot name="composer.left" />
        <Pickers />
        <View style={{ flex: 1 }} />
        <Slot name="composer.right" />
        <Button
          onpress={() => void composer.attach()}
          style={{ size: 28, radius: p.radius.s, align: "center", justify: "center", color: p.muted }}
          hover={{ background: p.hover, color: p.fg }}
        >
          <Icon svg={paperclip} size={16} />
        </Button>
        {#if daemon.status.running && composer.text.trim() === "" && composer.comments.length === 0}
          <Button
            onpress={() => void daemon.cancel()}
            style={{ size: 32, radius: 16, align: "center", justify: "center", background: p.danger, color: "#ffffff" }}
            hover={{ opacity: 0.9 }}
          >
            <Icon svg={stop} size={12} />
          </Button>
        {:else}
          <Button
            onpress={() => composer.submit()}
            style={{
              size: 32,
              radius: 16,
              align: "center",
              justify: "center",
              background: p.accent,
              color: "#ffffff",
              opacity: composer.text.trim() === "" && composer.attachments.length === 0 && composer.comments.length === 0 ? 0.64 : 1,
            }}
            hover={{ opacity: 0.9 }}
          >
            <Icon svg={arrowUp} size={14} />
          </Button>
        {/if}
      </View>
    {/if}
  </View>
</View>
