//! Everything else GitHub lets you add to an issue or pull request, from the detail: assignees
//! (yourself included), labels, milestone, type, projects, the organization's issue fields and
//! the relations between issues. Each is a short list to pick from (type to filter, Space to
//! toggle, Enter to apply) or a number to type, and one `gh` write when applied.
//!
//! `m` shows the keys; they work from the detail directly: `A` assignees, `L` labels, `M`
//! milestone, `T` type, `F` fields, `P` projects, `R` relations.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::Span;

use super::gh::Changes;
use super::model::{Kind, OrgField};
use super::view::chip_style;
use super::write::{Panel, Writing, row_line};
use super::{Github, Mode, Msg};
use crate::style::Styles;
use neo_github::fetch;

/// Most items of a list on screen.
const ROWS: usize = 8;

/// What can be fetched once and kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Which {
    Milestones,
    Types,
    Fields,
    Projects,
}

pub(super) enum Loaded {
    Names(Vec<String>),
    Fields(Vec<OrgField>),
}

/// What the owner and the repository offer, as far as it was asked for.
#[derive(Default)]
pub(super) struct Cache {
    milestones: Option<Vec<String>>,
    types: Option<Vec<String>>,
    fields: Option<Vec<OrgField>>,
    projects: Option<Vec<String>>,
}

pub(super) struct Item {
    pub value: String,
    pub label: String,
    pub color: Option<u32>,
    pub on: bool,
    pub was: bool,
}

#[derive(Clone)]
pub(super) enum What {
    Assignees,
    Labels,
    Milestone,
    Type,
    Projects,
    Field,
    FieldOption(OrgField),
    Link,
}

pub(super) struct Picker {
    pub what: What,
    pub title: String,
    pub items: Vec<Item>,
    pub filter: String,
    pub at: usize,
    pub multi: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Link {
    SetParent,
    RemoveParent,
    AddSub,
    RemoveSub,
    AddBlockedBy,
    RemoveBlockedBy,
    AddBlocking,
    RemoveBlocking,
}

const LINKS: [(Link, &str); 8] = [
    (Link::SetParent, "Set the parent issue"),
    (Link::RemoveParent, "Remove the parent issue"),
    (Link::AddSub, "Add a sub-issue"),
    (Link::RemoveSub, "Remove a sub-issue"),
    (Link::AddBlockedBy, "Add blocked by"),
    (Link::RemoveBlockedBy, "Remove blocked by"),
    (Link::AddBlocking, "Add blocking"),
    (Link::RemoveBlocking, "Remove blocking"),
];

/// What a typed value is for.
pub(super) enum Then {
    Title,
    FieldValue(OrgField),
    Link(Link),
}

pub(super) struct Input {
    pub prompt: String,
    pub text: String,
    pub then: Then,
}

fn names(values: &[String], current: &[String]) -> Vec<Item> {
    let mut items: Vec<Item> = values
        .iter()
        .map(|v| Item {
            value: v.clone(),
            label: v.clone(),
            color: None,
            on: current.contains(v),
            was: current.contains(v),
        })
        .collect();
    // Something the item has that the list does not (a closed milestone) is still shown.
    for c in current {
        if !values.contains(c) {
            items.push(Item {
                value: c.clone(),
                label: c.clone(),
                color: None,
                on: true,
                was: true,
            });
        }
    }
    items
}

impl Picker {
    /// The items that match what was typed, as indexes.
    fn shown(&self) -> Vec<usize> {
        let f = self.filter.to_lowercase();
        (0..self.items.len())
            .filter(|&i| self.items[i].label.to_lowercase().contains(&f))
            .collect()
    }

    fn move_by(&mut self, by: isize) {
        let n = self.shown().len();
        if n > 0 {
            self.at = (self.at as isize + by).rem_euclid(n as isize) as usize;
        }
    }
}

impl Github {
    fn owner(&self) -> Option<String> {
        self.repo_name()
            .and_then(|r| r.split('/').next())
            .map(str::to_string)
    }

    /// The keys that edit; true when it was one of them.
    pub(super) fn edit_hotkey(&mut self, c: char) -> bool {
        if self.shown_detail().is_none() {
            return true;
        }
        match c {
            'm' => self.writing = Some(Writing::Edit),
            'A' => self.start(What::Assignees),
            'L' => self.start(What::Labels),
            'M' => self.start(What::Milestone),
            'T' => self.start(What::Type),
            'F' => self.start(What::Field),
            'P' => self.start(What::Projects),
            'R' => self.start(What::Link),
            'S' => {
                let title = self.shown_detail().map(|d| d.title.clone()).unwrap_or_default();
                self.writing = Some(Writing::Input(Input {
                    prompt: "Title".into(),
                    text: title,
                    then: Then::Title,
                }));
            }
            _ => return false,
        }
        true
    }

    /// Opens the list for `what`, fetching what it needs first.
    pub(super) fn start(&mut self, what: What) {
        let which = match what {
            What::Milestone => Some(Which::Milestones),
            What::Type => Some(Which::Types),
            What::Field => Some(Which::Fields),
            What::Projects => Some(Which::Projects),
            _ => None,
        };
        if let Some(which) = which
            && !self.has(which)
        {
            self.fetch_options(which);
            self.writing = Some(Writing::Loading(what));
            return;
        }
        self.open_picker(what);
    }

    fn has(&self, which: Which) -> bool {
        match which {
            Which::Milestones => self.cache.milestones.is_some(),
            Which::Types => self.cache.types.is_some(),
            Which::Fields => self.cache.fields.is_some(),
            Which::Projects => self.cache.projects.is_some(),
        }
    }

    fn fetch_options(&mut self, which: Which) {
        let (Some(repo), Some(owner)) = (self.repo_name().map(str::to_string), self.owner()) else {
            return;
        };
        self.mine_asked += 1;
        self.spawn(move |gh, cwd| {
            let result = match which {
                Which::Milestones => fetch::milestones(gh, cwd, &repo).map(Loaded::Names),
                Which::Types => fetch::issue_types(gh, cwd, &owner).map(Loaded::Names),
                Which::Fields => fetch::org_fields(gh, cwd, &owner).map(Loaded::Fields),
                Which::Projects => fetch::projects(gh, cwd, &owner).map(Loaded::Names),
            };
            Msg::Loaded(which, result)
        });
    }

    /// What was asked for arrived (or could not): the list that waited for it opens.
    pub(super) fn loaded(&mut self, which: Which, result: Result<Loaded, super::GhError>) {
        self.mine_asked = self.mine_asked.saturating_sub(1);
        let what = match self.writing.take() {
            Some(Writing::Loading(what)) => Some(what),
            other => {
                self.writing = other;
                None
            }
        };
        match result {
            Ok(Loaded::Names(v)) => match which {
                Which::Milestones => self.cache.milestones = Some(v),
                Which::Types => self.cache.types = Some(v),
                _ => self.cache.projects = Some(v),
            },
            Ok(Loaded::Fields(v)) => self.cache.fields = Some(v),
            Err(e) => {
                let hint = match which {
                    Which::Projects => " (gh needs the project scope: gh auth refresh -s project)",
                    Which::Types | Which::Fields => " (only organizations have them)",
                    Which::Milestones => "",
                };
                self.note = Some(format!("could not load them: {}{hint}", e.message()));
                return;
            }
        }
        if let Some(what) = what {
            self.open_picker(what);
        }
    }

    fn shown_detail(&self) -> Option<&super::model::Detail> {
        let Mode::Detail(v) = &self.mode else {
            return None;
        };
        self.details.get(&(v.kind, v.number))?.detail.as_ref()
    }

    fn open_picker(&mut self, what: What) {
        let Some(d) = self.shown_detail() else {
            return;
        };
        let (title, items, multi) = match &what {
            What::Assignees => {
                let mut items = vec![Item {
                    value: "@me".into(),
                    label: "@me (yourself)".into(),
                    color: None,
                    on: false,
                    was: false,
                }];
                let logins: Vec<String> = self.people.iter().map(|p| p.login.clone()).collect();
                items.extend(names(&logins, &d.assignees));
                ("Assignees", items, true)
            }
            What::Labels => {
                let mut items: Vec<Item> = self
                    .labels
                    .iter()
                    .map(|l| Item {
                        value: l.name.clone(),
                        label: l.name.clone(),
                        color: Some(l.rgb),
                        on: d.labels.contains(&l.name),
                        was: d.labels.contains(&l.name),
                    })
                    .collect();
                for l in &d.labels {
                    if !self.labels.iter().any(|x| &x.name == l) {
                        items.push(Item {
                            value: l.clone(),
                            label: l.clone(),
                            color: None,
                            on: true,
                            was: true,
                        });
                    }
                }
                ("Labels", items, true)
            }
            What::Milestone => {
                let have = std::slice::from_ref(&d.milestone);
                let mut items = names(self.cache.milestones.as_deref().unwrap_or(&[]), have);
                items.insert(0, none_item(d.milestone.is_empty()));
                ("Milestone", items, false)
            }
            What::Type => {
                let have = std::slice::from_ref(&d.issue_type);
                let mut items = names(self.cache.types.as_deref().unwrap_or(&[]), have);
                items.insert(0, none_item(d.issue_type.is_empty()));
                ("Type", items, false)
            }
            What::Projects => {
                let items = names(self.cache.projects.as_deref().unwrap_or(&[]), &d.projects);
                ("Projects", items, true)
            }
            What::Field => {
                let current = self.current_fields();
                let items = self
                    .cache
                    .fields
                    .iter()
                    .flatten()
                    .map(|f| {
                        let value = current
                            .iter()
                            .find(|(n, _)| n == &f.name)
                            .map_or(String::new(), |(_, v)| format!(": {v}"));
                        Item {
                            value: f.name.clone(),
                            label: format!("{}{value}", f.name),
                            color: None,
                            on: false,
                            was: false,
                        }
                    })
                    .collect();
                ("Fields", items, false)
            }
            What::FieldOption(field) => {
                let current = self
                    .current_fields()
                    .into_iter()
                    .find(|(n, _)| n == &field.name)
                    .map(|(_, v)| vec![v])
                    .unwrap_or_default();
                let items = names(&field.options, &current);
                return self.finish_picker(
                    What::FieldOption(field.clone()),
                    field.name.clone(),
                    items,
                    false,
                );
            }
            What::Link => {
                let items = LINKS
                    .iter()
                    .enumerate()
                    .map(|(i, (_, label))| Item {
                        value: i.to_string(),
                        label: (*label).to_string(),
                        color: None,
                        on: false,
                        was: false,
                    })
                    .collect();
                ("Relations", items, false)
            }
        };
        self.finish_picker(what, title.to_string(), items, multi);
    }

    fn finish_picker(&mut self, what: What, title: String, items: Vec<Item>, multi: bool) {
        // Starts on the first one that is on, for a single choice.
        let at = if multi {
            0
        } else {
            items.iter().position(|i| i.on).unwrap_or(0)
        };
        self.writing = Some(Writing::Pick(Picker {
            what,
            title,
            items,
            filter: String::new(),
            at,
            multi,
        }));
    }

    /// The values of the issue fields of the shown item.
    fn current_fields(&self) -> Vec<(String, String)> {
        let Mode::Detail(v) = &self.mode else {
            return Vec::new();
        };
        self.details
            .get(&(v.kind, v.number))
            .map(|d| d.fields.clone())
            .unwrap_or_default()
    }

    /// A key while choosing from a list, typing a value, or looking at the keys.
    pub(super) fn edit_key(&mut self, writing: Writing, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match writing {
            Writing::Edit => {
                if key.code == KeyCode::Esc {
                    return;
                }
                // Any key that edits works from here too.
                if self.write_hotkey(key) {
                    return;
                }
                self.writing = Some(Writing::Edit);
            }
            Writing::Loading(what) => {
                if key.code != KeyCode::Esc {
                    self.writing = Some(Writing::Loading(what));
                }
            }
            Writing::Pick(mut picker) => {
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Up => picker.move_by(-1),
                    KeyCode::Down => picker.move_by(1),
                    KeyCode::Char('p') if ctrl => picker.move_by(-1),
                    KeyCode::Char('n') if ctrl => picker.move_by(1),
                    KeyCode::Backspace => {
                        picker.filter.pop();
                        picker.at = 0;
                    }
                    KeyCode::Char(' ') if picker.multi => {
                        let shown = picker.shown();
                        if let Some(&i) = shown.get(picker.at) {
                            picker.items[i].on = !picker.items[i].on;
                        }
                    }
                    KeyCode::Char(c) if !ctrl => {
                        picker.filter.push(c);
                        picker.at = 0;
                    }
                    KeyCode::Enter => {
                        self.chosen(picker);
                        return;
                    }
                    _ => {}
                }
                self.writing = Some(Writing::Pick(picker));
            }
            Writing::Input(mut input) => {
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Backspace => {
                        input.text.pop();
                    }
                    KeyCode::Char(c) if !ctrl => input.text.push(c),
                    KeyCode::Enter => {
                        self.typed(input);
                        return;
                    }
                    _ => {}
                }
                self.writing = Some(Writing::Input(input));
            }
            other => self.writing = Some(other),
        }
    }

    /// Enter on a list: apply what was toggled, or go on with the one chosen.
    fn chosen(&mut self, picker: Picker) {
        let Some((kind, number)) = self.target_pair() else {
            return;
        };
        if picker.multi {
            let added: Vec<String> = picker
                .items
                .iter()
                .filter(|i| i.on && !i.was)
                .map(|i| i.value.clone())
                .collect();
            let removed: Vec<String> = picker
                .items
                .iter()
                .filter(|i| !i.on && i.was)
                .map(|i| i.value.clone())
                .collect();
            let mut c = Changes::default();
            match picker.what {
                What::Assignees => (c.add_assignees, c.remove_assignees) = (added, removed),
                What::Labels => (c.add_labels, c.remove_labels) = (added, removed),
                _ => (c.add_projects, c.remove_projects) = (added, removed),
            }
            if c.is_empty() {
                self.info = Some("no change".into());
            } else {
                self.apply(kind, number, c, "updated");
            }
            return;
        }
        let shown = picker.shown();
        let Some(&i) = shown.get(picker.at) else {
            self.writing = Some(Writing::Pick(picker));
            return;
        };
        let value = picker.items[i].value.clone();
        match &picker.what {
            What::Milestone => {
                let c = Changes {
                    milestone: Some((!value.is_empty()).then_some(value)),
                    ..Changes::default()
                };
                self.apply(kind, number, c, "milestone set");
            }
            What::Type => {
                let c = Changes {
                    issue_type: Some((!value.is_empty()).then_some(value)),
                    ..Changes::default()
                };
                self.apply(kind, number, c, "type set");
            }
            What::Field => {
                let field = self
                    .cache
                    .fields
                    .iter()
                    .flatten()
                    .find(|f| f.name == value)
                    .cloned();
                match field {
                    Some(f) if f.data_type == "single_select" => {
                        self.open_picker(What::FieldOption(f));
                    }
                    Some(f) => {
                        self.writing = Some(Writing::Input(Input {
                            prompt: format!("{} ({})", f.name, f.data_type),
                            text: String::new(),
                            then: Then::FieldValue(f),
                        }));
                    }
                    None => {}
                }
            }
            What::FieldOption(field) => self.apply_field(kind, number, field.clone(), value),
            What::Link => {
                let link = LINKS[value.parse::<usize>().unwrap_or(0)].0;
                if link == Link::RemoveParent {
                    let c = Changes {
                        parent: Some(None),
                        ..Changes::default()
                    };
                    self.apply(kind, number, c, "parent removed");
                } else {
                    self.writing = Some(Writing::Input(Input {
                        prompt: format!("{} #", LINKS.iter().find(|l| l.0 == link).map_or("", |l| l.1)),
                        text: String::new(),
                        then: Then::Link(link),
                    }));
                }
            }
            _ => {}
        }
    }

    /// Enter on a typed value.
    fn typed(&mut self, input: Input) {
        let Some((kind, number)) = self.target_pair() else {
            return;
        };
        let text = input.text.trim().to_string();
        match input.then {
            Then::Title => {
                let same = self.shown_detail().is_some_and(|d| d.title == text);
                if text.is_empty() {
                    self.note = Some("a title needs some text".into());
                    self.writing = Some(Writing::Input(Input {
                        then: Then::Title,
                        ..input
                    }));
                } else if same {
                    self.info = Some("no change".into());
                } else {
                    let c = Changes {
                        title: Some(text),
                        ..Changes::default()
                    };
                    self.apply(kind, number, c, "title saved");
                }
            }
            Then::FieldValue(field) => {
                if text.is_empty() {
                    self.note = Some("a value".into());
                    self.writing = Some(Writing::Input(Input {
                        then: Then::FieldValue(field),
                        ..input
                    }));
                } else {
                    self.apply_field(kind, number, field, text);
                }
            }
            Then::Link(link) => match text.trim_start_matches('#').parse::<u64>() {
                Ok(n) if n > 0 && n != number => {
                    let mut c = Changes::default();
                    match link {
                        Link::SetParent => c.parent = Some(Some(n)),
                        Link::AddSub => c.add_sub_issues = vec![n],
                        Link::RemoveSub => c.remove_sub_issues = vec![n],
                        Link::AddBlockedBy => c.add_blocked_by = vec![n],
                        Link::RemoveBlockedBy => c.remove_blocked_by = vec![n],
                        Link::AddBlocking => c.add_blocking = vec![n],
                        Link::RemoveBlocking => c.remove_blocking = vec![n],
                        Link::RemoveParent => c.parent = Some(None),
                    }
                    self.apply(kind, number, c, "linked");
                }
                _ => {
                    self.note = Some("the number of another issue".into());
                    self.writing = Some(Writing::Input(Input {
                        then: Then::Link(link),
                        ..input
                    }));
                }
            },
        }
    }

    fn target_pair(&self) -> Option<(Kind, u64)> {
        match &self.mode {
            Mode::Detail(v) => Some((v.kind, v.number)),
            Mode::List => None,
        }
    }

    fn apply(&mut self, kind: Kind, number: u64, changes: Changes, done: &str) {
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        self.send(done, "could not change it", move |gh, cwd| {
            fetch::edit(gh, cwd, kind, &repo, number, &changes)
        });
    }

    fn apply_field(&mut self, _kind: Kind, number: u64, field: OrgField, value: String) {
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        self.send("field set", "could not set the field", move |gh, cwd| {
            fetch::set_field(gh, cwd, &repo, number, &field, &value)
        });
    }

    /// The lines of the edit panels: the keys, a loading line, a list, a typed value.
    pub(super) fn edit_panel(
        &self,
        writing: &Writing,
        number: u64,
        w: usize,
        h: usize,
        st: &Styles,
        bg: Style,
    ) -> Option<Panel> {
        let bold = Style::new().add_modifier(ratatui::style::Modifier::BOLD);
        let row = |spans: Vec<Span<'static>>| row_line(spans, w, bg);
        let key = |k: &str, what: &str| -> Vec<Span<'static>> {
            vec![
                Span::styled(k.to_string(), st.accent),
                Span::raw(format!(" {what}   ")),
            ]
        };
        Some(match writing {
            Writing::Edit => {
                // The keys fill rows as wide as the panel.
                let mut lines = vec![row(vec![Span::styled(format!("Edit #{number}"), bold)])];
                let mut keys: Vec<Span<'static>> = Vec::new();
                let mut used = 0;
                let entries = [
                    ("E", "text"),
                    ("S", "title"),
                    ("A", "assignees"),
                    ("L", "labels"),
                    ("M", "milestone"),
                    ("T", "type"),
                    ("F", "fields"),
                    ("P", "projects"),
                    ("R", "relations"),
                    ("esc", "cancel"),
                ];
                for (k, what) in entries {
                    let size = crate::util::width(k) + crate::util::width(what) + 4;
                    if used > 0 && used + size > w.saturating_sub(2) {
                        lines.push(row(std::mem::take(&mut keys)));
                        used = 0;
                    }
                    keys.extend(key(k, what));
                    used += size;
                }
                lines.push(row(keys));
                Panel { lines, cursor: None }
            }
            Writing::Loading(_) => Panel {
                lines: vec![row(vec![Span::styled("loading…  esc cancel", st.muted)])],
                cursor: None,
            },
            Writing::Pick(p) => {
                let shown = p.shown();
                let room = ROWS.min(h.saturating_sub(2)).max(1);
                let top = (p.at + 1).saturating_sub(room);
                let hint = if p.multi {
                    "  space toggle · enter apply · esc cancel"
                } else {
                    "  enter choose · esc cancel"
                };
                let mut lines = vec![row(vec![
                    Span::styled(p.title.clone(), bold),
                    Span::styled(format!("  {}", p.filter), st.accent),
                    Span::styled(hint, st.muted),
                ])];
                for (n, &i) in shown.iter().enumerate().skip(top).take(room) {
                    let item = &p.items[i];
                    let mut spans = vec![Span::styled(
                        if n == p.at { "› " } else { "  " },
                        st.accent,
                    )];
                    if p.multi {
                        spans.push(Span::raw(if item.on { "[x] " } else { "[ ] " }));
                    } else if item.on {
                        spans.push(Span::styled("● ", st.accent));
                    }
                    match item.color {
                        Some(rgb) => spans.push(Span::styled(
                            format!(" {} ", item.label),
                            chip_style(Some(rgb), st),
                        )),
                        None => spans.push(Span::raw(item.label.clone())),
                    }
                    lines.push(row(spans));
                }
                if shown.is_empty() {
                    lines.push(row(vec![Span::styled("nothing matches", st.muted)]));
                }
                Panel { lines, cursor: None }
            }
            Writing::Input(input) => {
                let shown = format!("{}: ", input.prompt);
                let col = 1 + crate::util::width(&shown) + crate::util::width(&input.text);
                Panel {
                    lines: vec![
                        row(vec![
                            Span::styled(shown, bold),
                            Span::raw(input.text.clone()),
                            Span::styled("   enter apply · esc cancel", st.muted),
                        ]),
                    ],
                    cursor: Some((0, col)),
                }
            }
            _ => return None,
        })
    }
}

fn none_item(on: bool) -> Item {
    Item {
        value: String::new(),
        label: "(none)".into(),
        color: None,
        on,
        was: on,
    }
}
