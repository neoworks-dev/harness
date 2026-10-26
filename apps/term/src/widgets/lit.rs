//! Highlighting on demand: a diff's lines are parsed on a thread only around what is on
//! screen, however long the file.
//!
//! A grammar's parse depends on every line before, so a far jump can't be exact at once. The
//! worker keeps the parses it has going (cursors) and serves a range from the nearest one that
//! is close enough; else from the hunk the range is in (a hunk starts over anyway), when that
//! is close; else it starts fresh a little above the range, which is right for nearly all code,
//! and marks those lines as guessed so an exact parse that passes by later replaces them.

use std::cell::Cell;
use std::sync::mpsc::{self, Receiver, Sender};

use neo_renderer_ratatui::syntax::{Class, Highlighter, SyntaxReference};

/// A cursor this far above a range is still worth running on: exact, and quick enough.
const LOOKAHEAD: usize = 3_000;
/// A fresh parse starts this many lines above the range it is for.
const WARMUP: usize = 200;
/// Lines highlighted between two looks for a newer request (and sent as one batch).
const BATCH: usize = 256;
/// Parses kept going.
const CURSORS: usize = 8;

/// The classes of a line's old and new version (empty for a side it doesn't have).
pub(crate) type Lit = (Vec<Class>, Vec<Class>);

/// One row of what is highlighted.
#[derive(Debug, Clone)]
pub(crate) enum Step {
    /// The parse starts over (a hunk, another file), with this grammar from here on.
    Reset(Option<&'static SyntaxReference>),
    /// A line: its text on the old side and on the new one.
    Line {
        old: Option<String>,
        new: Option<String>,
    },
    /// Nothing to highlight (a header, a note).
    Skip,
}

/// How good a line's classes are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Quality {
    None,
    Guessed,
    Exact,
}

/// The rows' classes as far as they are known, and the worker that finds the rest.
pub(crate) struct Lazy {
    lit: Vec<Option<Lit>>,
    ask: Sender<(usize, usize)>,
    got: Receiver<Vec<(usize, Lit)>>,
    /// The last range asked for: the same one isn't asked again.
    asked: Cell<Option<(usize, usize)>>,
}

impl Lazy {
    /// Starts the worker for `steps`, parsed with `grammar` until a reset says otherwise; None
    /// when no step would be highlighted.
    pub(crate) fn start(
        grammar: Option<&'static SyntaxReference>,
        steps: Vec<Step>,
    ) -> Option<Lazy> {
        let any = grammar.is_some() || steps.iter().any(|s| matches!(s, Step::Reset(Some(_))));
        if !any {
            return None;
        }
        let (ask, asks) = mpsc::channel::<(usize, usize)>();
        let (send, got) = mpsc::channel();
        let n = steps.len();
        std::thread::spawn(move || Worker::new(grammar, steps).run(&asks, &send));
        Some(Lazy {
            lit: vec![None; n],
            ask,
            got,
            asked: Cell::new(None),
        })
    }

    /// Rows `from..to` are (about to be) on screen.
    pub(crate) fn want(&self, from: usize, to: usize) {
        let to = to.min(self.lit.len());
        if from >= to || self.asked.get() == Some((from, to)) {
            return;
        }
        self.asked.set(Some((from, to)));
        let _ = self.ask.send((from, to));
    }

    /// Takes in what the worker found. True when anything arrived.
    pub(crate) fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok(batch) = self.got.try_recv() {
            for (i, lit) in batch {
                if let Some(slot) = self.lit.get_mut(i) {
                    *slot = Some(lit);
                }
            }
            any = true;
        }
        any
    }

    pub(crate) fn get(&self, i: usize) -> Option<&Lit> {
        self.lit.get(i)?.as_ref()
    }

    /// Waits until rows `from..to` are all known (or a second passed).
    #[cfg(test)]
    pub(crate) fn wait(&mut self, from: usize, to: usize) {
        self.want(from, to);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < until {
            self.poll();
            if (from..to.min(self.lit.len())).all(|i| self.lit[i].is_some()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

#[derive(Clone)]
struct Cursor {
    /// The next step to parse.
    at: usize,
    old: Option<Highlighter>,
    new: Option<Highlighter>,
    exact: bool,
}

struct Worker {
    grammar: Option<&'static SyntaxReference>,
    steps: Vec<Step>,
    done: Vec<Quality>,
    /// Indexes of the resets, in order.
    resets: Vec<usize>,
    cursors: Vec<Cursor>,
}

impl Worker {
    fn new(grammar: Option<&'static SyntaxReference>, steps: Vec<Step>) -> Worker {
        let resets = (0..steps.len())
            .filter(|&i| matches!(steps[i], Step::Reset(_)))
            .collect();
        Worker {
            grammar,
            done: vec![Quality::None; steps.len()],
            steps,
            resets,
            cursors: Vec::new(),
        }
    }

    fn run(mut self, asks: &Receiver<(usize, usize)>, send: &Sender<Vec<(usize, Lit)>>) {
        let mut next = None;
        loop {
            let (from, to) = match next.take() {
                Some(range) => range,
                None => match asks.recv() {
                    Ok(range) => range,
                    Err(_) => return,
                },
            };
            // Only the latest request matters.
            let (from, to) = asks.try_iter().last().unwrap_or((from, to));
            let Some(mut cursor) = self.cursor_for(from, to) else {
                continue;
            };
            let mut batch = Vec::new();
            while cursor.at < to {
                let i = cursor.at;
                let lit = self.step(&mut cursor);
                let quality = if cursor.exact {
                    Quality::Exact
                } else {
                    Quality::Guessed
                };
                // A fresh parse's first lines are its warm-up: not worth showing.
                if quality > self.done[i] && (cursor.exact || i >= from) {
                    self.done[i] = quality;
                    batch.push((i, lit));
                }
                if batch.len() >= BATCH {
                    if send.send(std::mem::take(&mut batch)).is_err() {
                        return;
                    }
                    if let Some(newer) = asks.try_iter().last() {
                        next = Some(newer);
                        break;
                    }
                }
            }
            if !batch.is_empty() && send.send(batch).is_err() {
                return;
            }
            self.keep(cursor);
        }
    }

    /// The parse to serve `from..to` with; None when everything there is as good as it gets.
    fn cursor_for(&mut self, from: usize, to: usize) -> Option<Cursor> {
        let to = to.min(self.steps.len());
        let first = (from..to).find(|&i| self.done[i] < Quality::Exact)?;
        // A running parse close enough above.
        if let Some(at) = self
            .cursors
            .iter()
            .enumerate()
            .filter(|(_, c)| c.at <= first && first - c.at <= LOOKAHEAD)
            .max_by_key(|(_, c)| c.at)
            .map(|(i, _)| i)
        {
            return Some(self.cursors.swap_remove(at));
        }
        // The start of the hunk it is in, when that is close: exact from there.
        let start = self.resets.iter().rev().find(|&&r| r <= first).copied();
        if first - start.unwrap_or(0) <= LOOKAHEAD {
            return Some(self.fresh(start.unwrap_or(0), true));
        }
        // Too far for an exact parse: a guess, unless the range has one.
        if (first..to).all(|i| self.done[i] > Quality::None) {
            return None;
        }
        let from_line = first.saturating_sub(WARMUP).max(start.map_or(0, |r| r + 1));
        Some(self.fresh(from_line, false))
    }

    /// A parse from scratch at step `at`, with the grammar in effect there.
    fn fresh(&self, at: usize, exact: bool) -> Cursor {
        let grammar = match self.resets.iter().rev().find(|&&r| r <= at) {
            Some(&r) => match self.steps[r] {
                Step::Reset(g) => g,
                _ => None,
            },
            None => self.grammar,
        };
        Cursor {
            at,
            old: grammar.map(Highlighter::new),
            new: grammar.map(Highlighter::new),
            exact,
        }
    }

    /// Parses the cursor's step and moves it on.
    fn step(&self, cursor: &mut Cursor) -> Lit {
        let i = cursor.at;
        cursor.at += 1;
        match &self.steps[i] {
            Step::Reset(grammar) => {
                cursor.old = grammar.map(Highlighter::new);
                cursor.new = grammar.map(Highlighter::new);
                // Whatever came before, from here on the parse is exact.
                cursor.exact = true;
                (Vec::new(), Vec::new())
            }
            Step::Line { old, new } => {
                let side = |h: &mut Option<Highlighter>, text: &Option<String>| match (h, text) {
                    (Some(h), Some(text)) => h.line(text),
                    _ => Vec::new(),
                };
                (side(&mut cursor.old, old), side(&mut cursor.new, new))
            }
            Step::Skip => (Vec::new(), Vec::new()),
        }
    }

    fn keep(&mut self, cursor: Cursor) {
        if cursor.at >= self.steps.len() {
            return;
        }
        self.cursors.retain(|c| c.at != cursor.at);
        self.cursors.push(cursor);
        if self.cursors.len() > CURSORS {
            // The one furthest up goes.
            if let Some(at) = (0..self.cursors.len()).min_by_key(|&i| self.cursors[i].at) {
                self.cursors.swap_remove(at);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_renderer_ratatui::syntax::syntax_for;

    fn lines(n: usize) -> Vec<Step> {
        (0..n)
            .map(|i| Step::Line {
                old: None,
                new: Some(format!("let x{i} = \"s\"; // c")),
            })
            .collect()
    }

    #[test]
    fn only_what_is_asked_for_is_highlighted() {
        let mut lazy = Lazy::start(syntax_for("a.rs"), lines(100_000)).unwrap();
        lazy.wait(50_000, 50_040);
        assert!(lazy.get(50_000).is_some());
        assert!(lazy.get(50_039).unwrap().1.contains(&Class::Keyword));
        // Nowhere near: untouched.
        assert!(lazy.get(10).is_none());
        assert!(lazy.get(90_000).is_none());
    }

    #[test]
    fn a_guess_is_replaced_by_the_exact_parse_that_passes_by() {
        // An open block comment far above makes everything after it a comment.
        let mut steps = vec![Step::Line {
            old: None,
            new: Some("/* open".into()),
        }];
        steps.extend(lines(6_000));
        let mut lazy = Lazy::start(syntax_for("a.rs"), steps).unwrap();
        lazy.wait(5_000, 5_010);
        let guess = lazy.get(5_005).unwrap().1.clone();
        assert!(guess.contains(&Class::Keyword), "a fresh parse can't know");
        // Walking down from the top reaches it exactly.
        for at in (0..=5_000).step_by(1_000) {
            lazy.wait(at, at + 10);
        }
        let comment = |l: &Lazy| l.get(5_005).unwrap().1.iter().all(|c| *c == Class::Comment);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !comment(&lazy) && std::time::Instant::now() < until {
            lazy.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(comment(&lazy), "{:?}", lazy.get(5_005));
    }

    #[test]
    fn a_hunk_starts_an_exact_parse_with_its_grammar() {
        let mut steps = lines(10_000);
        steps.push(Step::Reset(syntax_for("a.py")));
        steps.push(Step::Line {
            old: Some("def f(): pass".into()),
            new: None,
        });
        let mut lazy = Lazy::start(syntax_for("a.rs"), steps).unwrap();
        lazy.wait(10_001, 10_002);
        let (old, new) = lazy.get(10_001).unwrap();
        assert!(old.contains(&Class::Keyword) && new.is_empty());
        assert!(lazy.get(5_000).is_none());
    }

    #[test]
    fn nothing_to_highlight_starts_nothing() {
        assert!(Lazy::start(None, lines(3)).is_none());
    }
}
