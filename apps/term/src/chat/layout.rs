use super::*;

impl Chat {
    // ---- layout ----

    /// Lays the items out for `width` x `height`; reuses rendered lines of untouched items.
    pub fn layout(&mut self, width: u16, height: u16, opts: &ChatOpts) {
        self.height = usize::from(height);
        self.width = usize::from(width);
        let key = LayoutKey {
            width,
            opts: opts.clone(),
            generation: self.generation,
            images: self.images.clone(),
        };
        if self.layout_key.as_ref() == Some(&key) {
            self.apply_reveal();
            self.light_visible();
            return;
        }
        let was_total = self.total;
        let items = &self.state.items;
        let last = items.len().saturating_sub(1);
        // The items drawn, and whether each is the thinking line.
        let shown: Vec<(usize, bool)> = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                // A question the user allowed has done its job: the tool's own row says the rest.
                if is_allowed_question(item) {
                    return None;
                }
                // A choice (/settings, /model…) the user waved away doesn't stay in the chat.
                if item.r#ref.starts_with("q-")
                    && item.state == Some(ItemState::Pending)
                    && opts.dismissed.contains(&item.r#ref)
                {
                    return None;
                }
                // Hidden thoughts show while the agent thinks, as one animated line, and then go.
                let thinking = item.kind == ItemKind::Thought && !opts.show_thoughts;
                (!thinking || index == last).then_some((index, thinking))
            })
            .collect();
        let mut entries: Vec<Entry> = Vec::with_capacity(shown.len());
        let mut targets: Vec<Target> = Vec::new();
        let mut summaries = turn_changes(items, opts.running).into_iter().peekable();
        // With hide-edits only the newest edit shows what it changed (Ctrl-O opens them all).
        let last_edit = items.iter().rposition(is_edit);
        let open_edit = |index: usize| !opts.hide_edits || Some(index) == last_edit;
        // A blank row between entries, tool calls too.
        let mut offset = 0;
        let mut k = 0;
        loop {
            // A finished turn's changed files go after its last item.
            let next = shown.get(k).map(|&(index, _)| index);
            while let Some((end, changed)) =
                summaries.next_if(|(end, _)| next.is_none_or(|n| *end < n))
            {
                if !entries.is_empty() {
                    offset += 1;
                }
                let r = format!("{}#changes", items[end].r#ref);
                let lines = changes_lines(&changed, usize::from(width), opts, self.cwd.as_deref());
                for (j, c) in changed.iter().enumerate() {
                    targets.push(Target {
                        r#ref: r.clone(),
                        what: Pick::File(c.path.clone()),
                        line: offset + 1 + j,
                        len: 1,
                    });
                }
                let len = lines.len();
                entries.push(Entry {
                    r#ref: r,
                    thinking: false,
                    running: false,
                    lines: Rc::new(lines),
                    slots: Rc::new(Vec::new()),
                    focus: None,
                    offset,
                    lit: true,
                });
                offset += len;
            }
            if k >= shown.len() {
                break;
            }
            let (index, thinking) = shown[k];
            let item = &items[index];
            let r = &item.r#ref;
            let tool_row =
                item.kind == ItemKind::Tool && !self.buttons.contains_key(r) && !self.is_form(r);
            // A run of finished calls that something came after is folded to one line.
            if !opts.expanded && foldable(item, &self.buttons, &self.docs) {
                let end = (k..shown.len())
                    .find(|&j| !foldable(&items[shown[j].0], &self.buttons, &self.docs))
                    .unwrap_or(shown.len());
                if end - k >= FOLD_MIN && end < shown.len() {
                    let run: Vec<&ViewItem> =
                        shown[k..end].iter().map(|&(i, _)| &items[i]).collect();
                    if !entries.is_empty() {
                        offset += 1;
                    }
                    entries.push(Entry {
                        r#ref: r.clone(),
                        thinking: false,
                        running: false,
                        lines: Rc::new(vec![folded_line(&run, usize::from(width), &opts.palette)]),
                        slots: Rc::new(Vec::new()),
                        focus: None,
                        offset,
                        lit: true,
                    });
                    offset += 1;
                    k = end;
                    continue;
                }
            }
            k += 1;
            let rev = self.revs.get(r).copied().unwrap_or(0);
            let active = opts
                .active
                .as_ref()
                .filter(|(a, _)| a == r)
                .map(|(_, b)| *b);
            let ckey = CacheKey {
                rev,
                width,
                // What a read read stays folded, unless it is a picture. An edit keeps its
                // preview: the whole file is the file view's (Enter or Ctrl-O on its row).
                expanded: opts.expanded
                    && (!self.reads.contains(r) || self.sources.contains_key(r))
                    && !is_edit(item),
                thoughts: opts.show_thoughts,
                icons: opts.icons,
                split_diffs: opts.split_diffs,
                open_edit: opts.expanded || open_edit(index),
                focused: active,
                linked: opts.linked.as_ref() == Some(r),
                palette: opts.palette,
                marker: opts.marker,
                images: self
                    .sources
                    .contains_key(r)
                    .then(|| self.images.clone())
                    .flatten(),
                ui: self
                    .docs
                    .contains_key(r)
                    .then(|| opts.ui.get(r).cloned().unwrap_or_default()),
            };
            let (lines, slots, focus, lit, refs) = match self.cache.get(r) {
                // Ctrl-O (everything in full) keeps the whole card, e.g. a long diff.
                _ if opts.in_panel
                    && active.is_some()
                    && !opts.expanded
                    && !self.docs.contains_key(r) =>
                {
                    (
                        Rc::new(vec![panel_stub(item, opts)]),
                        Rc::new(Vec::new()),
                        None,
                        true,
                        Rc::new(Vec::new()),
                    )
                }
                Some(c) if c.key == ckey => (
                    c.lines.clone(),
                    c.slots.clone(),
                    c.focus,
                    c.lit,
                    c.refs.clone(),
                ),
                // Without syntax colors: they are worked out for what comes into view.
                _ => {
                    let (app, cwd) = (self.app.as_deref(), self.cwd.as_deref());
                    let (lines, slots, focus) = render(item, width, &ckey, app, cwd, false);
                    let refs = ref_lines(&neoui_refs(item), &lines);
                    let (lines, slots, refs) = (Rc::new(lines), Rc::new(slots), Rc::new(refs));
                    self.cache.insert(
                        r.clone(),
                        Cached {
                            key: ckey,
                            lines: lines.clone(),
                            slots: slots.clone(),
                            focus,
                            lit: false,
                            refs: refs.clone(),
                        },
                    );
                    (lines, slots, focus, false, refs)
                }
            };
            if lines.is_empty() {
                continue;
            }
            if !entries.is_empty() {
                offset += 1;
            }
            let len = lines.len();
            if tool_row && let Some(first) = changed_files(item).into_iter().next() {
                targets.push(Target {
                    r#ref: r.clone(),
                    what: Pick::File(first.path),
                    line: offset,
                    len: 1,
                });
            } else if item.kind == ItemKind::Agent && !self.has_targets(r) {
                targets.push(Target {
                    r#ref: r.clone(),
                    what: Pick::Reply,
                    line: offset,
                    len,
                });
            }
            // A card's ref is picked by its first line; what the item lets the keys reach
            // (fields, buttons) takes them instead.
            if !self.has_targets(r) {
                targets.extend(refs.iter().map(|(at, dev)| Target {
                    r#ref: r.clone(),
                    what: Pick::Ref(dev.clone()),
                    line: offset + at,
                    len: 1,
                }));
            }
            entries.push(Entry {
                r#ref: r.clone(),
                thinking,
                running: tool_row && item.state == Some(ItemState::Running),
                lines,
                slots,
                focus,
                offset,
                lit,
            });
            offset += len;
        }
        let live = items.len();
        if self.cache.len() > live {
            let items = &self.state.items;
            self.cache
                .retain(|r, _| items.iter().any(|i| &i.r#ref == r));
        }
        self.entries = entries;
        self.targets = targets;
        if self.selected_index().is_none() {
            self.selected = None;
        }
        self.palette = opts.palette;
        self.total = offset;
        self.layout_key = Some(key);
        if let Some((r, within)) = self.anchor.take()
            && self.top.is_some()
        {
            if let Some(top) = self.anchored_top(&r, within) {
                self.top = Some(top);
            }
        } else if self.top.is_some() && self.total > was_total {
            self.unseen = true;
        }
        if let Some(top) = self.top {
            self.top = Some(top.min(self.max_top()));
        }
        self.apply_reveal();
        self.light_visible();
    }

    /// Gives the code of the entries in view (and a screen above and below) its syntax colors.
    /// The lines stay where they are: colors don't change where a line wraps. Highlighting a
    /// whole long conversation at once (Ctrl-O) took a noticeable moment.
    pub(super) fn light_visible(&mut self) {
        let (top, height) = (self.top(), self.height.max(1));
        let (from, to) = (top.saturating_sub(height), top + 2 * height);
        let first = self
            .entries
            .partition_point(|e| e.offset + e.lines.len() <= from);
        let (app, cwd) = (self.app.as_deref(), self.cwd.as_deref());
        for e in &mut self.entries[first..] {
            if e.offset >= to {
                break;
            }
            if e.lit {
                continue;
            }
            e.lit = true;
            let item = self.state.items.iter().rev().find(|i| i.r#ref == e.r#ref);
            let (Some(item), Some(c)) = (item, self.cache.get_mut(&e.r#ref)) else {
                continue;
            };
            if c.lit {
                (e.lines, e.slots, e.focus) = (c.lines.clone(), c.slots.clone(), c.focus);
                continue;
            }
            let (lines, slots, focus) = render(item, c.key.width, &c.key, app, cwd, true);
            // Should colors ever move a line, the plain lines stay: the layout counted them.
            if lines.len() != e.lines.len() {
                continue;
            }
            (c.lines, c.slots, c.focus, c.lit) = (Rc::new(lines), Rc::new(slots), focus, true);
            (e.lines, e.slots, e.focus) = (c.lines.clone(), c.slots.clone(), c.focus);
        }
    }

    pub fn total_lines(&self) -> usize {
        self.total
    }

    pub(super) fn max_top(&self) -> usize {
        self.total.saturating_sub(self.height)
    }

    /// The first visible line.
    pub fn top(&self) -> usize {
        self.top.unwrap_or_else(|| self.max_top())
    }

    pub fn following(&self) -> bool {
        self.top.is_none()
    }

    pub fn has_unseen(&self) -> bool {
        self.unseen
    }

    /// The lines in view (at most the pane's height), the thinking line still.
    pub fn visible(&self) -> Vec<Line<'static>> {
        self.visible_at(None)
    }

    /// The lines in view, the thinking line and running calls animated for the time `now_ms`.
    pub fn visible_at(&self, now_ms: Option<i64>) -> Vec<Line<'static>> {
        let top = self.top();
        let bottom = top + self.height;
        let mut out: Vec<Line<'static>> = Vec::with_capacity(self.height);
        let first = self
            .entries
            .partition_point(|e| e.offset + e.lines.len() <= top);
        let picked = self
            .selected_index()
            .filter(|_| self.highlight)
            .map(|i| &self.targets[i]);
        let picked_file = picked.filter(|t| t.what != Pick::Reply).map(|t| t.line);
        let picked_reply = picked
            .filter(|t| t.what == Pick::Reply)
            .map(|t| t.line..t.line + t.len);
        let mut at = top;
        for e in &self.entries[first..] {
            if e.offset >= bottom {
                break;
            }
            while at < e.offset {
                out.push(Line::default());
                at += 1;
            }
            let skip = at - e.offset;
            for line in e.lines.iter().skip(skip) {
                if at >= bottom {
                    break;
                }
                match now_ms {
                    Some(now) if e.thinking => out.push(thinking_line(line, now, &self.palette)),
                    Some(now) if e.running && at == e.offset => {
                        let lit = sweep(line, now, &self.palette);
                        let ran = self
                            .started
                            .get(&e.r#ref)
                            .map(|started| now - started)
                            .filter(|ms| *ms >= ELAPSED_AFTER_MS)
                            .map(crate::agents::elapsed);
                        let background = self.background_hint.as_deref() == Some(e.r#ref.as_str());
                        let hint = match (ran, background) {
                            (Some(ran), true) => format!("  {ran} · {BACKGROUND_HINT}"),
                            (Some(ran), false) => format!("  {ran}"),
                            (None, true) => format!("  {BACKGROUND_HINT}"),
                            (None, false) => String::new(),
                        };
                        out.push(if hint.is_empty() {
                            lit
                        } else {
                            with_hint(lit, &hint, self.width, &self.palette)
                        });
                    }
                    _ if picked_file == Some(at) => out.push(picked_line(line, self.width)),
                    _ if picked_reply.as_ref().is_some_and(|r| r.contains(&at)) => {
                        let first = picked_reply.as_ref().is_some_and(|r| r.start == at);
                        out.push(picked_reply_line(line, first, self.width, &self.palette));
                    }
                    _ => out.push(line.clone()),
                }
                at += 1;
            }
        }
        out
    }
}
