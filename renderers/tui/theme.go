package tui

import protocol "neoworks.dev/suite/protocol/go"

// DefaultTheme is protocol/themes/default.json, for callers whose daemon
// has not sent a theme.
func DefaultTheme() protocol.Theme {
	return protocol.Theme{
		Name: "default-dark",
		Colors: protocol.ThemeColors{
			Fg: "#e6e6e6", Bg: "#16181d", Surface: "#1f2229", Border: "#343844",
			Accent: "#7aa2f7", Muted: "#8a8f9c", Success: "#9ece6a", Danger: "#f7768e",
		},
		Spacing: protocol.ThemeSpacing{S: 4, M: 8, L: 16},
		Text: protocol.ThemeText{
			Title: protocol.TextStyle{Size: 15, Weight: 600},
			Body:  protocol.TextStyle{Size: 14, Weight: 400},
			Code:  protocol.TextStyle{Size: 13, Weight: 400, Mono: true},
		},
	}
}
