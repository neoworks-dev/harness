//! Type-to-filter on a long choice (`/model` with a key for every provider): which options
//! match what was typed, and where the focus goes among them. Pure, so it is tested alone.

use neo_renderer_ratatui::ButtonRef;

/// The number keys stop at 9: a choice with more options than that gets a filter field.
pub const FILTER_FROM: usize = 9;

/// Whether the buttons are a choice long enough to filter. Permission cards never are.
pub fn is_long(buttons: &[ButtonRef]) -> bool {
    buttons.len() > FILTER_FROM && buttons.iter().all(|b| b.action == "neo.choose")
}

/// The option's id, as the daemon knows it (`openrouter/anthropic/claude-haiku-4.5`).
fn option_id(button: &ButtonRef) -> &str {
    button
        .args
        .as_ref()
        .and_then(|a| a.get("option"))
        .and_then(|o| o.as_str())
        .unwrap_or("")
}

/// Whether every word of the filter occurs, in any case, in the option's label, id or
/// description.
pub fn matches(button: &ButtonRef, filter: &str) -> bool {
    let haystack = format!(
        "{}\n{}\n{}",
        button.label,
        option_id(button),
        button.description.as_deref().unwrap_or("")
    )
    .to_lowercase();
    filter
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

/// The positions of the options that match, in their order.
pub fn matching(buttons: &[ButtonRef], filter: &str) -> Vec<usize> {
    (0..buttons.len())
        .filter(|&i| matches(&buttons[i], filter))
        .collect()
}

/// The focus after moving `delta` places among the matches, wrapping at the ends. Focus on
/// something that is no match goes to the nearest end in the direction of the move.
pub fn step(matches: &[usize], focused: usize, delta: isize) -> usize {
    let n = matches.len() as isize;
    if n == 0 {
        return focused;
    }
    match matches.iter().position(|&m| m == focused) {
        Some(at) => matches[(at as isize + delta).rem_euclid(n) as usize],
        None if delta >= 0 => matches[0],
        None => matches[matches.len() - 1],
    }
}

/// Where the focus goes when the filter changed: the first match, or with an empty filter the
/// option in effect now (else the first). None when nothing matches.
pub fn refocus(buttons: &[ButtonRef], filter: &str) -> Option<usize> {
    if filter.trim().is_empty() {
        return Some(buttons.iter().position(|b| b.current).unwrap_or(0));
    }
    matching(buttons, filter).first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn option(label: &str, id: &str, description: Option<&str>) -> ButtonRef {
        ButtonRef {
            label: label.into(),
            action: "neo.choose".into(),
            args: Some(json!({"id": "c", "option": id})),
            key: "1".into(),
            current: false,
            description: description.map(str::to_string),
            note: false,
            stays: false,
            input: None,
            deadline: None,
        }
    }

    fn models() -> Vec<ButtonRef> {
        vec![
            option("Claude Haiku 4.5 · claude", "claude/haiku", None),
            option(
                "Claude Haiku 4.5 · openrouter",
                "openrouter/anthropic/claude-haiku-4.5",
                Some("Fast and cheap"),
            ),
            option("GPT 5 · openrouter", "openrouter/openai/gpt-5", None),
            option("GPT 5 · openai", "openai/gpt-5", Some("OpenAI flagship")),
        ]
    }

    #[test]
    fn only_a_choice_with_more_than_nine_options_filters() {
        let mut nine: Vec<_> = (0..9)
            .map(|i| option("m", &format!("m{i}"), None))
            .collect();
        assert!(!is_long(&nine));
        nine.push(option("m", "m9", None));
        assert!(is_long(&nine));
        nine[3].action = "neo.permit".into();
        assert!(!is_long(&nine));
    }

    #[test]
    fn every_word_must_occur_in_the_label_the_id_or_the_description() {
        let m = models();
        assert_eq!(matching(&m, ""), [0, 1, 2, 3]);
        assert_eq!(matching(&m, "OPENROUTER"), [1, 2]);
        assert_eq!(matching(&m, "openrouter haiku"), [1]);
        // The id has what the label lacks, and the description counts too.
        assert_eq!(matching(&m, "anthropic"), [1]);
        assert_eq!(matching(&m, "flagship gpt"), [3]);
        assert_eq!(matching(&m, "  gpt   5 "), [2, 3]);
        assert!(matching(&m, "gemini").is_empty());
    }

    #[test]
    fn the_focus_moves_over_the_matches_only() {
        let hits = [1, 2, 5];
        assert_eq!(step(&hits, 1, 1), 2);
        assert_eq!(step(&hits, 5, 1), 1);
        assert_eq!(step(&hits, 1, -1), 5);
        // Off the matches it lands on an end.
        assert_eq!(step(&hits, 3, 1), 1);
        assert_eq!(step(&hits, 3, -1), 5);
        assert_eq!(step(&[], 3, 1), 3);
    }

    #[test]
    fn a_new_filter_focuses_the_first_match_and_an_empty_one_the_current_option() {
        let mut m = models();
        m[2].current = true;
        assert_eq!(refocus(&m, "haiku"), Some(0));
        assert_eq!(refocus(&m, "openai"), Some(2));
        assert_eq!(refocus(&m, ""), Some(2));
        assert_eq!(refocus(&m, "nope"), None);
    }
}
