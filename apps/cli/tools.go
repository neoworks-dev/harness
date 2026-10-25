package main

import (
	"context"
	"fmt"
	"io"
	"slices"
	"strconv"
	"strings"
	"unicode/utf8"

	protocol "neoworks.dev/suite/protocol/go"
)

// defaultWidth is the line width for tool descriptions when $COLUMNS is unset.
const defaultWidth = 100

func runTools(ctx context.Context, args []string, e cliEnv) int {
	fs, socket := commandFlags("tools", e)
	positional, err := parseFlags(fs, args)
	if err != nil {
		return 2
	}
	words := strings.Fields(strings.Join(positional, " ")) // a quoted query counts as several words
	c, _, ok := connect(ctx, *socket, e)
	if !ok {
		return 1
	}
	defer c.Close()

	connectors, err := c.ListConnectors(ctx)
	if err != nil {
		fmt.Fprintf(e.Stderr, "suite: %v\n", err)
		return 1
	}
	if len(connectors) == 0 {
		fmt.Fprintln(e.Stdout, "no connectors")
		return 0
	}
	width, _ := strconv.Atoi(e.Getenv("COLUMNS"))
	if width <= 0 {
		width = defaultWidth
	}
	if !printConnectors(e.Stdout, connectors, words, width) {
		fmt.Fprintf(e.Stdout, "no tools match %q\n", strings.Join(words, " "))
	}
	return 0
}

// printConnectors writes each connector with its tools. With query words, it
// keeps only matching tools and the connectors that have any. It reports
// whether anything was printed.
func printConnectors(w io.Writer, connectors []protocol.ConnectorInfo, query []string, width int) bool {
	slices.SortStableFunc(connectors, func(a, b protocol.ConnectorInfo) int { return strings.Compare(a.App, b.App) })
	printed := false
	for _, c := range connectors {
		var tools []protocol.ConnectorTool
		for _, t := range c.Tools {
			if matchesAll(t, query) {
				tools = append(tools, t)
			}
		}
		if len(query) > 0 && len(tools) == 0 {
			continue
		}
		printed = true
		state := "closed"
		if c.Connected {
			state = "connected"
		}
		head := fmt.Sprintf("%s (%s, %s, v%d)", c.App, c.Kind, state, c.V)
		if c.Error != nil && *c.Error != "" {
			head += " error: " + oneLine(*c.Error)
		}
		fmt.Fprintln(w, head)
		if len(tools) == 0 {
			fmt.Fprintln(w, "  (no tools)")
		}
		for _, t := range tools {
			fmt.Fprintln(w, toolLine(t, width))
		}
	}
	return printed
}

// matchesAll reports whether the tool's name or description contains every
// query word, ignoring case.
func matchesAll(t protocol.ConnectorTool, query []string) bool {
	text := strings.ToLower(t.Name + " " + t.Description)
	for _, word := range query {
		if !strings.Contains(text, strings.ToLower(word)) {
			return false
		}
	}
	return true
}

// toolLine formats "  name — description", cutting the description so the
// line fits in width columns.
func toolLine(t protocol.ConnectorTool, width int) string {
	line := "  " + t.Name
	desc := oneLine(t.Description)
	if desc == "" {
		return line
	}
	line += " — "
	room := width - utf8.RuneCountInString(line)
	switch {
	case room <= 1:
		return strings.TrimRight(line, " —")
	case utf8.RuneCountInString(desc) > room:
		desc = string([]rune(desc)[:room-1]) + "…"
	}
	return line + desc
}

// oneLine collapses all whitespace, newlines included, to single spaces.
func oneLine(s string) string { return strings.Join(strings.Fields(s), " ") }
