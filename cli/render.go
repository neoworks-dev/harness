package main

import (
	"fmt"
	"io"
	"maps"
	"slices"
	"strings"

	protocol "neoworks.dev/suite/protocol/go"
)

// maxToolLines is how many lines of tool output are shown before truncating.
const maxToolLines = 10

// renderer prints session events as plain text. It remembers whether the
// output sits at the start of a line and which message was streamed last, so
// chunks of one message join up and anything else starts on its own line.
type renderer struct {
	w           io.Writer
	thoughts    bool   // show agent_thought_chunk
	atLineStart bool   //
	lastStream  string // kind and messageId of the last streamed chunk
}

func newRenderer(w io.Writer, thoughts bool) *renderer {
	return &renderer{w: w, thoughts: thoughts, atLineStart: true}
}

func (r *renderer) write(s string) {
	if s == "" {
		return
	}
	io.WriteString(r.w, s) // a broken stdout has nowhere to be reported
	r.atLineStart = strings.HasSuffix(s, "\n")
}

func (r *renderer) printf(format string, args ...any) { r.write(fmt.Sprintf(format, args...)) }

func (r *renderer) freshLine() {
	if !r.atLineStart {
		r.write("\n")
	}
}

// line prints s on a line of its own.
func (r *renderer) line(format string, args ...any) {
	r.freshLine()
	r.printf(format+"\n", args...)
}

// Render prints one event. Events it does not know are ignored.
func (r *renderer) Render(ev protocol.SessionEvent) {
	switch e := ev.Value.(type) {
	case protocol.UserMessage:
		r.freshLine()
		r.printf("\n» %s: %s\n", e.App, e.Text)
		for _, img := range e.Images {
			r.printf("  [image: %s]\n", img)
		}
		r.write("\n")
	case protocol.AgentMessageChunk:
		r.stream("message", e.MessageID, "", e.Text)
	case protocol.AgentThoughtChunk:
		if r.thoughts {
			r.stream("thought", e.MessageID, "… ", e.Text)
		}
	case protocol.ToolCall:
		r.line("⚙ %s", e.Title)
	case protocol.ToolCallUpdate:
		r.toolUpdate(e)
	case protocol.TurnEnded:
		r.freshLine()
		if e.StopReason != protocol.StopReasonEndTurn {
			if e.Error != nil && *e.Error != "" {
				r.printf("[turn %s: %s]\n", e.StopReason, *e.Error)
			} else {
				r.printf("[turn %s]\n", e.StopReason)
			}
		}
	case protocol.AppConnected:
		if len(e.Tools) == 0 {
			r.line("[app connected: %s (%s v%d)]", e.App, e.Kind, e.V)
		} else {
			r.line("[app connected: %s (%s v%d): %s]", e.App, e.Kind, e.V, strings.Join(e.Tools, ", "))
		}
	case protocol.AppDisconnected:
		r.line("[app disconnected: %s]", e.App)
	case protocol.FocusChanged:
		s := "[focus: " + e.App
		for _, k := range slices.Sorted(maps.Keys(e.Context)) {
			s += " " + k + "=" + e.Context[k]
		}
		r.line("%s]", s)
	}
}

// stream prints a chunk of a streamed message. A different message than the
// previous chunk's starts on a fresh line; prefix is repeated on every line.
func (r *renderer) stream(kind, messageID, prefix, text string) {
	key := kind + "/" + messageID
	if key != r.lastStream {
		r.freshLine()
		r.lastStream = key
	}
	if prefix == "" {
		r.write(text)
		return
	}
	for text != "" {
		if r.atLineStart {
			r.write(prefix)
		}
		line, rest, found := strings.Cut(text, "\n")
		if found {
			line += "\n"
		}
		r.write(line)
		text = rest
	}
}

// toolUpdate reports finished tool calls only; progress updates are noise in
// a text transcript.
func (r *renderer) toolUpdate(u protocol.ToolCallUpdate) {
	if u.Status == nil {
		return
	}
	switch *u.Status {
	case protocol.ToolCallStatusCompleted:
		r.line("  ✓ done")
	case protocol.ToolCallStatusFailed:
		r.line("  ✗ failed")
	default:
		return
	}
	for _, c := range u.Content {
		switch c := c.Value.(type) {
		case protocol.TextContent:
			r.toolText(c.Text)
		case protocol.DiffContent:
			old := ""
			if c.OldText != nil {
				old = *c.OldText
			}
			added, removed := diffCounts(old, c.NewText)
			r.printf("    diff %s (+%d -%d lines)\n", c.Path, added, removed)
		}
	}
}

func (r *renderer) toolText(text string) {
	text = strings.TrimRight(text, "\n")
	if text == "" {
		return
	}
	lines := strings.Split(text, "\n")
	for _, l := range lines[:min(len(lines), maxToolLines)] {
		r.printf("    %s\n", l)
	}
	if more := len(lines) - maxToolLines; more > 0 {
		r.printf("    … (%d more lines)\n", more)
	}
}

// diffCounts approximates a diff by dropping the lines the two texts share at
// both ends and counting the rest.
func diffCounts(oldText, newText string) (added, removed int) {
	oldLines, newLines := splitLines(oldText), splitLines(newText)
	for len(oldLines) > 0 && len(newLines) > 0 && oldLines[0] == newLines[0] {
		oldLines, newLines = oldLines[1:], newLines[1:]
	}
	for len(oldLines) > 0 && len(newLines) > 0 && oldLines[len(oldLines)-1] == newLines[len(newLines)-1] {
		oldLines, newLines = oldLines[:len(oldLines)-1], newLines[:len(newLines)-1]
	}
	return len(newLines), len(oldLines)
}

func splitLines(s string) []string {
	if s == "" {
		return nil
	}
	return strings.Split(strings.TrimSuffix(s, "\n"), "\n")
}
