//! The shown session's details at the foot of the sessions sidebar: the directory it works in,
//! its agent's context (share and exact tokens), the tokens in and out and the cost so far, and
//! the language servers and MCP servers (the daemon's `service` connectors) with whether each is
//! up.

use neo_protocol::{ConnectorInfo, ConnectorKind, Usage};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::style::Styles;
use crate::util::{fit_line, width};
use crate::widgets::{format_cost, truncate_start};

/// What the block shows.
pub struct Details<'a> {
    /// The session's directory, `~` for the home directory.
    pub dir: &'a str,
    pub usage: Option<&'a Usage>,
    pub connectors: &'a [ConnectorInfo],
}

/// `1234567` as `1,234,567`.
pub fn grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// A heading: bold on a filled pane, else with a rule to the edge: ` Session ───────`.
fn heading(name: &str, w: usize, styles: &Styles) -> Line<'static> {
    if styles.fills() {
        return Line::from(Span::styled(
            format!(" {name}"),
            Style::new().add_modifier(Modifier::BOLD),
        ));
    }
    let rule = "─".repeat(w.saturating_sub(width(name) + 2));
    Line::from(vec![
        Span::styled(
            format!(" {name} "),
            styles.muted.add_modifier(Modifier::BOLD),
        ),
        Span::styled(rule, styles.border),
    ])
}

/// The block's lines in `w` columns.
pub fn lines(d: &Details, w: usize, styles: &Styles) -> Vec<Line<'static>> {
    let mut out = vec![heading("Session", w, styles)];
    out.push(Line::from(vec![
        Span::raw(" "),
        Span::raw(truncate_start(d.dir, w.saturating_sub(2))),
    ]));
    if let Some(u) = d.usage {
        if let (Some(used), Some(size)) = (u.context_used, u.context_size)
            && size > 0
        {
            let share = (used.max(0) as f64 / size as f64 * 100.0).round() as i64;
            let tone = if share >= 85 {
                styles.danger
            } else if share >= 60 {
                styles.accent
            } else {
                styles.muted
            };
            out.push(Line::from(vec![
                Span::styled(format!(" ctx {share}%"), tone),
                Span::styled(
                    format!("  {} / {}", grouped(used), grouped(size)),
                    styles.muted,
                ),
            ]));
        }
        if u.input_tokens.is_some() || u.output_tokens.is_some() {
            out.push(Line::from(Span::styled(
                format!(
                    " in {} · out {}",
                    grouped(u.input_tokens.unwrap_or(0)),
                    grouped(u.output_tokens.unwrap_or(0))
                ),
                styles.muted,
            )));
        }
        if let Some(cost) = u.cost {
            out.push(Line::from(Span::styled(
                format!(" {}", format_cost(cost)),
                styles.muted,
            )));
        }
    }
    let mut servers: Vec<&ConnectorInfo> = d
        .connectors
        .iter()
        .filter(|c| c.kind == ConnectorKind::Service)
        .collect();
    servers.sort_by(|a, b| a.app.cmp(&b.app));
    if !servers.is_empty() {
        out.push(heading("Servers", w, styles));
        for c in servers {
            let up = c.connected && c.error.is_none();
            let (dot, tone) = if up {
                ("●", styles.success)
            } else {
                ("○", styles.danger)
            };
            let mut spans = vec![
                Span::styled(format!(" {dot} "), tone),
                Span::raw(c.app.clone()),
            ];
            let note = match (&c.error, c.connected) {
                (Some(e), _) => Some(e.clone()),
                (None, false) => Some("disconnected".into()),
                (None, true) => Some(format!("{} tools", c.tools.len())),
            };
            if let Some(note) = note {
                spans.push(Span::styled(format!("  {note}"), styles.muted));
            }
            out.push(Line::from(spans));
        }
    }
    out.into_iter().map(|l| fit_line(l, w)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_renderer_ratatui::Palette;
    use serde_json::json;

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .map(|l| l.trim_end().to_string())
            .collect()
    }

    #[test]
    fn the_directory_numbers_and_servers_of_the_session() {
        let usage: Usage = serde_json::from_value(json!({
            "contextUsed": 108312, "contextSize": 200000, "cost": 2.333,
            "inputTokens": 1234567, "outputTokens": 45678
        }))
        .unwrap();
        let connectors: Vec<ConnectorInfo> = serde_json::from_value(json!([
            {"app": "terminal", "kind": "app", "connected": true, "v": 0, "tools": []},
            {"app": "mcp-headroom", "kind": "service", "connected": true, "v": 1,
             "tools": [{"name": "a", "description": ""}]},
            {"app": "lsp", "kind": "service", "connected": false, "v": 2, "tools": []}
        ]))
        .unwrap();
        let d = Details {
            dir: "~/Documents/neoworks/neoworks-harness",
            usage: Some(&usage),
            connectors: &connectors,
        };
        let styles = Styles::new(&Palette::default(), false);
        assert_eq!(
            text(&lines(&d, 40, &styles)),
            [
                " Session ───────────────────────────────",
                " ~/Documents/neoworks/neoworks-harness",
                " ctx 54%  108,312 / 200,000",
                " in 1,234,567 · out 45,678",
                " $2.33",
                " Servers ───────────────────────────────",
                " ○ lsp  disconnected",
                " ● mcp-headroom  1 tools",
            ]
        );
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
    }
}
