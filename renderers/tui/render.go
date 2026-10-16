package tui

import (
	"encoding/json"
	"strconv"
	"strings"

	protocol "neoworks.dev/suite/protocol/go"
)

// Options configure a render.
type Options struct {
	// Width is the number of terminal columns to draw in; 0 means 80.
	Width int
	// Color is how much color to use. The zero value, None, emits no escape
	// codes.
	Color ColorMode
	// Frame picks the glyph of the spinner shown for running tools, so a
	// caller redrawing a live item can animate it. Any value is valid.
	Frame int
	// Thoughts draws thought items in full (muted) instead of collapsed to
	// one "… thinking" line.
	Thoughts bool
}

const (
	defaultWidth = 80
	minWidth     = 10
)

func (o Options) width() int {
	switch {
	case o.Width == 0:
		return defaultWidth
	case o.Width < minWidth:
		return minWidth
	}
	return o.Width
}

// Render draws items one after the other, separated by a blank line. The
// result ends with a newline unless items is empty.
func Render(items []protocol.ViewItem, theme protocol.Theme, opts Options) string {
	var b strings.Builder
	for i, it := range items {
		if i > 0 {
			b.WriteByte('\n')
		}
		b.WriteString(RenderItem(it, theme, opts))
		b.WriteByte('\n')
	}
	return b.String()
}

// RenderItem draws one item, without a trailing newline.
func RenderItem(item protocol.ViewItem, theme protocol.Theme, opts Options) string {
	r := &renderer{opts: opts, pal: newPalette(theme, opts.Color), btns: Buttons(item)}
	return serialize(r.item(item), r.pal)
}

// Button is a button of an item as the terminal presents it.
type Button struct {
	// Key is what triggers it: the button's own key, else its 1-based
	// position among the item's buttons.
	Key    string
	Label  string
	Action string
	Args   json.RawMessage
}

// CopyText returns the text to copy when the button is a copy: action, which
// the client handles itself instead of sending ui.action.
func (b Button) CopyText() (string, bool) {
	return strings.CutPrefix(b.Action, "copy:")
}

// Buttons lists the item's buttons in reading order, with the keys the
// renderer shows for them.
func Buttons(item protocol.ViewItem) []Button {
	var out []Button
	collectButtons(item.Component, &out)
	return out
}

func collectButtons(c protocol.Component, out *[]Button) {
	switch v := c.Value.(type) {
	case protocol.ButtonComponent:
		key := strconv.Itoa(len(*out) + 1)
		if v.Key != nil && *v.Key != "" {
			key = *v.Key
		}
		*out = append(*out, Button{Key: key, Label: v.Label, Action: v.Action, Args: v.Args})
	case protocol.CardComponent:
		collectChildren(v.Children, out)
	case protocol.RowComponent:
		collectChildren(v.Children, out)
	case protocol.StackComponent:
		collectChildren(v.Children, out)
	}
}

func collectChildren(cs []protocol.Component, out *[]Button) {
	for _, c := range cs {
		collectButtons(c, out)
	}
}

// renderer holds what one draw needs.
type renderer struct {
	opts Options
	pal  *palette
	btns []Button // the item's buttons, handed out in order as they are drawn
	next int
}

var spinner = [...]string{"◐", "◓", "◑", "◒"}

func (r *renderer) item(it protocol.ViewItem) []line {
	w := r.opts.width()
	label := ""
	if it.Label != nil {
		label = clean(*it.Label)
	}
	switch it.Kind {
	case protocol.ItemKindUser:
		head := lit("»", style{fg: rAccent, bold: true})
		if label != "" {
			head = append(head, run{" " + label, style{fg: rAccent, bold: true}})
		}
		return append(wrapRuns(head, w), r.component(it.Component, w)...)
	case protocol.ItemKindAgent:
		// A guest agent's items carry its name; the main agent's have no label and no head.
		if label == "" {
			break
		}
		head := lit(label, style{fg: rAccent, bold: true})
		return append(wrapRuns(head, w), r.component(it.Component, w)...)
	case protocol.ItemKindThought:
		if !r.opts.Thoughts {
			if label != "" {
				return []line{lit("… "+label+" thinking", style{fg: rMuted})}
			}
			return []line{lit("… thinking", style{fg: rMuted})}
		}
		return tint(r.component(it.Component, w), rMuted)
	case protocol.ItemKindTool:
		head := r.toolHeader(it, label)
		return append(wrapRuns(head, w), indented(r.component(it.Component, w-2), 2)...)
	case protocol.ItemKindNotice:
		fg := rMuted
		if it.State != nil && *it.State == protocol.ItemStateFailed {
			fg = rDanger
		}
		return tint(r.component(it.Component, w), fg)
	}
	return r.component(it.Component, w)
}

func (r *renderer) toolHeader(it protocol.ViewItem, label string) line {
	var icon line
	if it.State != nil {
		switch *it.State {
		case protocol.ItemStatePending:
			icon = lit("○", style{fg: rMuted})
		case protocol.ItemStateRunning:
			icon = lit(spinner[uint(r.opts.Frame)%uint(len(spinner))], style{fg: rAccent})
		case protocol.ItemStateDone:
			icon = lit("✓", style{fg: rSuccess})
		case protocol.ItemStateFailed:
			icon = lit("✗", style{fg: rDanger})
		}
	}
	if label != "" {
		if len(icon) > 0 {
			icon = append(icon, run{" ", style{}})
		}
		icon = append(icon, run{label, style{bold: true}})
	}
	return icon
}
