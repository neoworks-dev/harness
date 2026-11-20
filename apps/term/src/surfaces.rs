//! Surfaces (`docs/neoui.md`): neoui programs of the session that stay, drawn in their place
//! rather than in the chat. The terminal draws `bottom` and `side` ones in a strip over the input,
//! each under its title, and a `page` in the chat's place until Esc. A page that opens is shown
//! at once; one put away comes back with `/surface <name>` or Tab to the strip's line for it.

use std::collections::{HashMap, HashSet};

use neo_protocol::{SurfacePlace, ViewItem};
use neo_renderer_ratatui::{Options, Palette, UiState, render_item};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// What a surface is called: its title (the item's label), else its name.
pub fn title_of(surface: &ViewItem) -> String {
    surface
        .label
        .clone()
        .unwrap_or_else(|| surface.r#ref.trim_start_matches("s-").to_string())
}

/// One entry of the strip over the input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Entry<'a> {
    /// A bar or a panel, drawn in full.
    Surface(&'a ViewItem),
    /// A page not shown: one line that brings it back (Enter while it has the keys).
    Page(&'a ViewItem),
}

impl<'a> Entry<'a> {
    pub fn item(self) -> &'a ViewItem {
        match self {
            Entry::Surface(s) | Entry::Page(s) => s,
        }
    }
}

/// The strip over the input: the bars, then the panels, as they opened; then a line for each
/// page that is not the one shown.
pub fn strip<'a>(surfaces: &'a [ViewItem], shown: Option<&str>) -> Vec<Entry<'a>> {
    let of = |place: SurfacePlace| surfaces.iter().filter(move |s| s.place == Some(place));
    of(SurfacePlace::Bottom)
        .chain(of(SurfacePlace::Side))
        .map(Entry::Surface)
        .chain(
            of(SurfacePlace::Page)
                .filter(|p| Some(p.r#ref.as_str()) != shown)
                .map(Entry::Page),
        )
        .collect()
}

/// The line of a page not shown.
pub fn page_line(page: &ViewItem, style: Style) -> Line<'static> {
    Line::from(Span::styled(format!("▸ {} (page)", title_of(page)), style))
}

/// Which page has the chat's place.
#[derive(Debug, Default)]
pub struct Pages {
    /// The ref of the page shown; none shows the chat.
    pub shown: Option<String>,
    seen: HashSet<String>,
    session: String,
}

impl Pages {
    /// The surfaces changed: a page that opened since is shown at once, the one shown stays while
    /// it is open, else the chat. Another session starts from what it has open, shown or not.
    pub fn sync(&mut self, session: &str, surfaces: &[ViewItem]) {
        let pages: Vec<&ViewItem> = surfaces
            .iter()
            .filter(|s| s.place == Some(SurfacePlace::Page))
            .collect();
        if session != self.session {
            self.session = session.to_string();
            self.seen = surfaces.iter().map(|s| s.r#ref.clone()).collect();
            self.shown = None;
            return;
        }
        if let Some(fresh) = pages.iter().rev().find(|p| !self.seen.contains(&p.r#ref)) {
            self.shown = Some(fresh.r#ref.clone());
        } else if !pages.iter().any(|p| Some(&p.r#ref) == self.shown.as_ref()) {
            self.shown = None;
        }
        self.seen = surfaces.iter().map(|s| s.r#ref.clone()).collect();
    }
}

/// A surface's lines at `width`: its title in the muted colour, then its doc, drawn with the
/// visual state the forms keep for it.
pub fn lines(
    surface: &ViewItem,
    width: u16,
    palette: Palette,
    states: HashMap<String, UiState>,
    title: Style,
) -> Vec<Line<'static>> {
    let opts = Options {
        theme: palette,
        neoui: states,
        ..Options::default()
    };
    let mut out = vec![Line::from(Span::styled(
        title_of(surface),
        title.add_modifier(Modifier::BOLD),
    ))];
    out.extend(render_item(surface, width.max(1), &opts));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn surface(name: &str, place: &str) -> ViewItem {
        serde_json::from_value(json!({
            "ref": format!("s-{name}"), "kind": "tool", "place": place,
            "component": {"type": "neoui", "ui": {"id": "0"}}
        }))
        .unwrap()
    }

    #[test]
    fn the_strip_has_the_bars_then_the_panels_then_the_pages_not_shown() {
        let all = [
            surface("a", "side"),
            surface("b", "page"),
            surface("c", "bottom"),
            surface("d", "page"),
        ];
        let refs = |shown| -> Vec<(String, bool)> {
            strip(&all, shown)
                .iter()
                .map(|e| (e.item().r#ref.clone(), matches!(e, Entry::Page(_))))
                .collect()
        };
        let named = |list: &[(&str, bool)]| -> Vec<(String, bool)> {
            list.iter().map(|(r, p)| ((*r).to_string(), *p)).collect()
        };
        assert_eq!(
            refs(Some("s-b")),
            named(&[("s-c", false), ("s-a", false), ("s-d", true)])
        );
        assert_eq!(refs(None).len(), 4);
    }

    #[test]
    fn a_page_that_opens_is_shown_and_one_put_away_stays_away() {
        let mut pages = Pages::default();
        pages.sync("s", &[]);
        pages.sync("s", &[surface("vars", "page")]);
        assert_eq!(pages.shown.as_deref(), Some("s-vars"));
        pages.shown = None;
        pages.sync("s", &[surface("vars", "page")]);
        assert_eq!(pages.shown, None);
        pages.sync("s", &[surface("vars", "page"), surface("chart", "page")]);
        assert_eq!(pages.shown.as_deref(), Some("s-chart"));
        pages.sync("s", &[surface("vars", "page")]);
        assert_eq!(pages.shown, None);
        // Another session: what it has open is not new.
        pages.sync("t", &[surface("x", "page")]);
        assert_eq!(pages.shown, None);
    }

    #[test]
    fn a_surface_is_called_by_its_title_else_its_name() {
        let mut s = surface("run", "bottom");
        assert_eq!(title_of(&s), "run");
        s.label = Some("Training".into());
        assert_eq!(title_of(&s), "Training");
    }
}
