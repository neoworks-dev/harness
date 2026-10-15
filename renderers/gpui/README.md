# renderers/gpui

Rust renderer (`suite-renderer-gpui`) that draws the resolved view format with GPUI. A reusable crate: Pad hosts it today, Grove will. Conventions shared by all renderers are in `../README.md`.

```rust
let chat = cx.new(ChatView::new);
cx.subscribe(&chat, |host, _, event: &ChatEvent, cx| match event {
    ChatEvent::Action { item, action, args } => { /* client.action(session, item, action, args) */ }
    ChatEvent::Copied(_) => {}                 // already on the clipboard
    ChatEvent::Leave => { /* focus your input */ }
})
.detach();

// Feed it what the SDK delivers:
//   Event::ViewSnapshot(s) -> chat.apply_snapshot(s, cx)
//   Event::ViewPatch(p)    -> chat.apply_patch(p, cx)
//   Event::Theme(t)        -> chat.set_theme(&t, cx)
// and call client.subscribe_view(session) / unsubscribe_view(session) / chat.clear(cx) yourself.
```

## Two layers

- `layout` (pure, no GPUI element code): `layout_item(&ViewItem, &ChatTheme) -> ItemLayout`, a tree of boxes, text runs (colors as theme tokens with their values, spacing in pixels), code, diff, table, kv, progress, button, image and file nodes, plus the item's buttons with their key hints. `dump_items` prints it as text; that is what `fixtures/<name>/gpui.txt` holds.
- `ChatView` (GPUI entity): `ViewState` + theme + one cached layout per item, drawn through GPUI's `list` aligned to the bottom (only visible items are built).

## `ChatView`

- `apply_snapshot` / `apply_patch` / `set_theme` / `clear`; `set_fonts(ChatFonts)` (default: system UI font, DejaVu Sans Mono / Menlo / Consolas), `set_remote_images(bool)` (off, see below).
- A patch re-lays-out only the items it touched, tells the list to re-measure only those, and ignores patches for another session or with a `seq` below the snapshot's. A snapshot keeps the layout of items that did not change, so a reconnect keeps scroll position and selection. The list stays at the end while it was at the end; `scroll_to_bottom()` pulls it back.
- Keyboard, while the chat has focus (`chat.focus_handle(cx)`; clicking an item with buttons focuses it): Up/Down move the selection between items with buttons (ring in `accent`; Up with nothing selected starts at the last one), a button's `key` (or its 1-based position when it has none; first match wins) presses it, Escape or Down past the last item emits `ChatEvent::Leave`. `focus_last_actionable(window, cx)` is what an input line calls on Up. Clicking a button presses it. `copy:<text>` writes to the clipboard and flashes the button; everything else is `ChatEvent::Action`.
- Thoughts are collapsed to `… thinking`; click the header to open. Tool items in the `running` state pulse.

## Images

`data:image/…;base64,` sources are decoded and drawn. http(s) sources are drawn only after `set_remote_images(true)` and only if the app has an HTTP client (GPUI apps start without one); until then, like `asset://`, paths and every other scheme, they show the fallback `[image W×H · src]` box. Nothing else is fetched or read.

## Not done

Text can't be selected (GPUI has no text selection yet), code isn't syntax-highlighted, diffs are unified (not side by side), pixel screenshots don't exist yet.

## Tests

`cargo test -p suite-renderer-gpui`: fixture snapshots (`UPDATE_SNAPSHOTS=1` rewrites `fixtures/*/gpui.txt`), layout/key/copy unit tests, and headless GPUI tests (patches, selection, clipboard, drawing every fixture, following the end).
