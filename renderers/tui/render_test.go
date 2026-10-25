package tui

import (
	"encoding/json"
	"strings"
	"testing"

	protocol "neoworks.dev/neo/protocol/go"
)

func TestWidth(t *testing.T) {
	tests := []struct {
		s    string
		want int
	}{
		{"", 0},
		{"abc", 3},
		{"日本語", 6},
		{"a日b", 4},
		{"📄 x", 4},
		{"✓ ✗ ▓░ » …", 10},     // narrow symbols the renderer draws with
		{"e\u0301", 1},         // combining accent
		{"👨\u200d👩\u200d👧", 2}, // a ZWJ sequence is one emoji
		{"👍🏽", 2},              // skin tone modifier
		{"한글", 4},
		{"ｱ", 1}, // halfwidth katakana
		{"Ａ", 2}, // fullwidth latin
	}
	for _, tc := range tests {
		if got := Width(tc.s); got != tc.want {
			t.Errorf("Width(%q) = %d, want %d", tc.s, got, tc.want)
		}
	}
}

func TestTruncateAndSplit(t *testing.T) {
	tests := []struct {
		s    string
		w    int
		want string
	}{
		{"hello", 5, "hello"},
		{"hello", 4, "hel…"},
		{"hello", 1, "…"},
		{"日本語日本語", 7, "日本語…"},
		{"日本語日本語", 6, "日本…"}, // …would make 7: the wide rune is dropped
		{"ab", 0, ""},
	}
	for _, tc := range tests {
		got := truncate(tc.s, tc.w)
		if got != tc.want || Width(got) > tc.w {
			t.Errorf("truncate(%q, %d) = %q, want %q", tc.s, tc.w, got, tc.want)
		}
	}
	if h, rest := splitWidth("日本語", 5); h != "日本" || rest != "語" {
		t.Errorf("splitWidth = %q, %q", h, rest)
	}
	if h, _ := splitWidth("日本", 1); h != "日" { // progress even when nothing fits
		t.Errorf("splitWidth with no room = %q", h)
	}
}

func TestCleanDropsEscapes(t *testing.T) {
	got := clean("a\x1b[31mred\x1b]52;c;AAAA\x07\r\nb\tc\u202ex")
	want := "a[31mred]52;c;AAAA\nb   cx"
	if got != want {
		t.Errorf("clean = %q, want %q", got, want)
	}
}

func TestParseHexAndNearest(t *testing.T) {
	if c, ok := parseHex("#7aa2f7"); !ok || c != (rgb{0x7a, 0xa2, 0xf7}) {
		t.Errorf("parseHex = %v, %v", c, ok)
	}
	if c, ok := parseHex("#abc"); !ok || c != (rgb{0xaa, 0xbb, 0xcc}) {
		t.Errorf("short hex = %v, %v", c, ok)
	}
	for _, bad := range []string{"", "#12", "#gggggg", "red"} {
		if _, ok := parseHex(bad); ok {
			t.Errorf("parseHex(%q) succeeded", bad)
		}
	}

	// The default theme in 16 colors: pastels keep their hue.
	th := DefaultTheme().Colors
	tests := []struct {
		name, hex string
		want      int
	}{
		{"accent", th.Accent, 12}, // bright blue
		{"success", th.Success, 2},
		{"danger", th.Danger, 9},
		{"muted", th.Muted, 8}, // bright black
		{"fg", th.Fg, 7},
		{"surface", th.Surface, 0},
		{"pure red", "#cd0000", 1},
		{"pure white", "#ffffff", 15},
		{"yellow-ish", "#e5c07b", 3},
	}
	for _, tc := range tests {
		c, _ := parseHex(tc.hex)
		if got := nearest16(c, -1); got != tc.want {
			t.Errorf("nearest16(%s %s) = %d, want %d", tc.name, tc.hex, got, tc.want)
		}
	}
	// A skipped color gives way to a neighbor.
	black, _ := parseHex(th.Border)
	if got := nearest16(black, -1); got != 0 {
		t.Errorf("dark border = %d, want black", got)
	}
	if got := nearest16(black, 0); got != 8 {
		t.Errorf("dark border without black = %d, want bright black", got)
	}

	// 256 colors: pastel red lands in the cube, grays on the ramp.
	tests256 := []struct {
		hex  string
		want int
	}{
		{th.Danger, 210}, // #ff8787
		{th.Accent, 111}, // #87afff
		{"#808080", 244}, // gray ramp
		{"#ffffff", 231}, // cube white
		{"#000000", 16},  // cube black, not palette entry 0
	}
	for _, tc := range tests256 {
		c, _ := parseHex(tc.hex)
		if got := nearest256(c, -1); got != tc.want {
			t.Errorf("nearest256(%s) = %d, want %d", tc.hex, got, tc.want)
		}
	}
}

func TestPaletteSGR(t *testing.T) {
	theme := DefaultTheme()
	tests := []struct {
		mode     ColorMode
		fg, bg   string // accent as foreground, surface as background
		borderFg string
	}{
		{None, "", "", ""},
		{ANSI16, "94", "40", "90"}, // the dark border avoids the black background
		{ANSI256, "38;5;111", "48;5;235", "38;5;237"},
		{TrueColor, "38;2;122;162;247", "48;2;31;34;41", "38;2;52;56;68"},
	}
	for _, tc := range tests {
		p := newPalette(theme, tc.mode)
		if p.fg[rAccent] != tc.fg || p.bg[rSurface] != tc.bg || p.fg[rBorder] != tc.borderFg {
			t.Errorf("%v: accent %q surface %q border %q; want %q %q %q",
				tc.mode, p.fg[rAccent], p.bg[rSurface], p.fg[rBorder], tc.fg, tc.bg, tc.borderFg)
		}
	}
}

func TestNoColorEmitsNoEscapes(t *testing.T) {
	for _, name := range fixtureNames(t) {
		out := Render(loadFixture(t, name), DefaultTheme(), Options{Width: 60})
		if strings.Contains(out, "\x1b") {
			t.Errorf("%s: escape code in uncolored output:\n%q", name, out)
		}
	}
}

func TestColorModesStyleTheSameText(t *testing.T) {
	items := loadFixture(t, "export-progress")
	plain := Render(items, DefaultTheme(), Options{Width: 60})
	for _, mode := range []ColorMode{ANSI16, ANSI256, TrueColor} {
		colored := Render(items, DefaultTheme(), Options{Width: 60, Color: mode})
		if !strings.Contains(colored, "\x1b[") {
			t.Fatalf("%v: no escape codes", mode)
		}
		if got := stripSGR(colored); got != plain {
			t.Errorf("%v: text differs once escapes are removed:\n%s\n--- want ---\n%s", mode, got, plain)
		}
	}
}

// stripSGR removes color escapes; output made by this package has no others.
func stripSGR(s string) string {
	var b strings.Builder
	for i := 0; i < len(s); i++ {
		if s[i] == 0x1b {
			for i < len(s) && s[i] != 'm' {
				i++
			}
			continue
		}
		b.WriteByte(s[i])
	}
	return b.String()
}

// render draws one component as a tool-less agent item.
func render(t *testing.T, c protocol.ComponentVariant, width int) string {
	t.Helper()
	item := protocol.ViewItem{Ref: "x", Kind: protocol.ItemKindAgent, Component: protocol.Component{Value: c}}
	return RenderItem(item, DefaultTheme(), Options{Width: width})
}

func TestTableCutsWidestColumnsFirst(t *testing.T) {
	tbl := protocol.TableComponent{
		Columns: []string{"id", "name", "note"},
		Rows: [][]string{
			{"1", "short", strings.Repeat("n", 40)},
			{"2", strings.Repeat("m", 30), "ok"},
		},
	}
	// Natural width is 2+30+40 plus 10 for the frame; at 40 the two wide
	// columns are cut evenly and "id" stays whole.
	got := render(t, tbl, 40)
	for _, l := range strings.Split(got, "\n") {
		if Width(l) > 40 {
			t.Errorf("line wider than 40: %q", l)
		}
	}
	want := strings.Join([]string{
		"┌────┬────────────────┬────────────────┐",
		"│ id │ name           │ note           │",
		"├────┼────────────────┼────────────────┤",
		"│ 1  │ short          │ nnnnnnnnnnnnn… │",
		"│ 2  │ mmmmmmmmmmmmm… │ ok             │",
		"└────┴────────────────┴────────────────┘",
	}, "\n")
	if got != want {
		t.Errorf("table:\n%s\nwant:\n%s", got, want)
	}

	// Plenty of room: nothing is cut.
	if got := render(t, tbl, 100); strings.Contains(got, "…") {
		t.Errorf("table cut at 100 columns:\n%s", got)
	}
}

func TestTableWideCharsAndTinyWidths(t *testing.T) {
	tbl := protocol.TableComponent{
		Columns: []string{"名前", "size"},
		Rows:    [][]string{{"日本語のファイル名", "1 KB"}, {"a\nb", ""}},
	}
	for _, w := range []int{10, 14, 20, 30, 80} {
		out := render(t, tbl, w)
		for _, l := range strings.Split(out, "\n") {
			if Width(l) > w {
				t.Errorf("width %d: line %q is %d wide", w, l, Width(l))
			}
		}
	}
	// Box edges stay aligned with wide characters in cells.
	lines := strings.Split(render(t, tbl, 30), "\n")
	for _, l := range lines {
		if Width(l) != Width(lines[0]) {
			t.Errorf("ragged table: %q is %d, first line %d", l, Width(l), Width(lines[0]))
		}
	}
}

func TestTableBlockWrapsCellsAndRulesTheRows(t *testing.T) {
	cell := func(text string) protocol.TableCell {
		return protocol.TableCell{Spans: []protocol.Span{{Text: text}}}
	}
	tbl := protocol.TableBlock{
		Header: []protocol.TableCell{cell("step"), cell("what happens")},
		Rows: [][]protocol.TableCell{
			{cell("1"), cell("the daemon parses the markdown")},
			{cell("2"), cell("it draws")},
		},
		Align: []protocol.ColumnAlign{protocol.ColumnAlignRight},
	}
	text := protocol.TextComponent{Blocks: []protocol.Block{{Value: tbl}}}
	want := strings.Join([]string{
		"┌──────┬──────────────────┐",
		"│ step │ what happens     │",
		"├──────┼──────────────────┤",
		"│    1 │ the daemon       │",
		"│      │ parses the       │",
		"│      │ markdown         │",
		"├──────┼──────────────────┤",
		"│    2 │ it draws         │",
		"└──────┴──────────────────┘",
	}, "\n")
	if got := render(t, text, 27); got != want {
		t.Errorf("table:\n%s\nwant:\n%s", got, want)
	}
	for _, w := range []int{10, 11, 12, 16, 40} { // 10 is the narrowest the renderer draws
		for _, l := range strings.Split(render(t, text, w), "\n") {
			if Width(l) > w {
				t.Errorf("width %d: line %q is %d wide", w, l, Width(l))
			}
		}
	}
}

func TestWrapKeepsWideCharsWithinWidth(t *testing.T) {
	spans := []protocol.Span{{Text: "日本語のテキストを折り返す test with ascii words 混在 and a-very-long-unbreakable-token-here"}}
	for _, w := range []int{10, 11, 17, 33} {
		for _, l := range wrapRuns(spanRuns(spans), w) {
			if l.width() > w {
				t.Errorf("width %d: %v is %d wide", w, l, l.width())
			}
		}
	}
}

func TestWrapKeepsStylesAcrossLines(t *testing.T) {
	bold := style{bold: true}
	lines := wrapRuns([]run{{"one two ", style{}}, {"three four", bold}}, 9)
	var got []string
	for _, l := range lines {
		var parts []string
		for _, r := range l {
			parts = append(parts, r.text)
			if r.st == bold {
				parts[len(parts)-1] = "*" + r.text + "*"
			}
		}
		got = append(got, strings.Join(parts, ""))
	}
	want := []string{"one two", "*three*", "*four*"}
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Errorf("wrapped = %q, want %q", got, want)
	}
}

func TestLinksShowTheirURL(t *testing.T) {
	href := "https://example.com/x"
	p := protocol.ParagraphBlock{Spans: []protocol.Span{
		{Text: "docs", Href: &href},
		{Text: " and "},
		{Text: href, Href: &href},
	}}
	txt := protocol.TextComponent{Blocks: []protocol.Block{{Value: p}}}
	got := render(t, txt, 80)
	if want := "docs (https://example.com/x) and https://example.com/x"; got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestProgressBar(t *testing.T) {
	label := "Working"
	tests := []struct {
		value float64
		want  string
	}{
		{0, "░░░░░░░░░░░░░░░░░░░░ 0% Working"},
		{0.6, "▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░ 60% Working"},
		{1, "▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓ 100% Working"},
		{1.7, "▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓ 100% Working"},
		{-1, "░░░░░░░░░░░░░░░░░░░░ 0% Working"},
	}
	for _, tc := range tests {
		got := render(t, protocol.ProgressComponent{Value: tc.value, Label: &label}, 80)
		if got != tc.want {
			t.Errorf("progress %v = %q, want %q", tc.value, got, tc.want)
		}
	}
	// Narrow: the label moves to its own line and the bar shrinks to fit.
	if got := render(t, protocol.ProgressComponent{Value: 0.5, Label: &label}, 20); got != "▓▓▓▓▓▓▓▓░░░░░░░░ 50%\nWorking" {
		t.Errorf("narrow progress = %q", got)
	}
}

func TestKVAlignsValues(t *testing.T) {
	kv := protocol.KvComponent{Items: []protocol.KvItem{
		{Key: "Format", Value: "PNG"},
		{Key: "Quality", Value: "a long value that has to wrap onto the next line, aligned"},
	}}
	got := render(t, kv, 30)
	want := "Format   PNG\n" +
		"Quality  a long value that has\n" +
		"         to wrap onto the next\n" +
		"         line, aligned"
	if got != want {
		t.Errorf("kv:\n%s\nwant:\n%s", got, want)
	}
}

func TestCardsCollapseWhenNarrow(t *testing.T) {
	title := "Title"
	card := protocol.CardComponent{Title: &title, Children: []protocol.Component{
		{Value: protocol.LinkComponent{URL: "https://x.y"}},
	}}
	if got := render(t, card, 30); !strings.HasPrefix(got, "╭─ Title ") || !strings.HasSuffix(got, "╯") {
		t.Errorf("card at 30:\n%s", got)
	}
	if got, want := render(t, card, 19), "Title\n  https://x.y"; got != want {
		t.Errorf("card at 19 = %q, want %q", got, want)
	}
}

func TestRowStacksUnlessAllButtons(t *testing.T) {
	btn := func(label string, key *string) protocol.Component {
		return protocol.Component{Value: protocol.ButtonComponent{Label: label, Action: "a.b", Key: key}}
	}
	k := "x"
	buttons := protocol.RowComponent{Children: []protocol.Component{btn("One", nil), btn("Two", &k), btn("Three", nil)}}
	if got, want := render(t, buttons, 80), "[1] One  [x] Two  [3] Three"; got != want {
		t.Errorf("button row = %q, want %q", got, want)
	}
	if got, want := render(t, buttons, 20), "[1] One\n[x] Two\n[3] Three"; got != want {
		t.Errorf("narrow button row = %q, want %q", got, want)
	}
	mixed := protocol.RowComponent{Children: []protocol.Component{
		btn("One", nil),
		{Value: protocol.LinkComponent{URL: "https://x.y"}},
	}}
	if got, want := render(t, mixed, 80), "[1] One\nhttps://x.y"; got != want {
		t.Errorf("mixed row = %q, want %q", got, want)
	}
}

func TestButtonsEffectiveKeys(t *testing.T) {
	var items []protocol.ViewItem
	data := `[{"ref":"r","kind":"tool","state":"done","component":{"type":"card","children":[
		{"type":"button","label":"A","action":"x.a"},
		{"type":"stack","children":[{"type":"button","label":"B","action":"copy:hello","key":"c"}]},
		{"type":"row","children":[{"type":"button","label":"C","action":"x.c","args":{"n":1}}]}]}}]`
	if err := json.Unmarshal([]byte(data), &items); err != nil {
		t.Fatal(err)
	}
	got := Buttons(items[0])
	if len(got) != 3 || got[0].Key != "1" || got[1].Key != "c" || got[2].Key != "3" {
		t.Fatalf("buttons = %+v", got)
	}
	if text, ok := got[1].CopyText(); !ok || text != "hello" {
		t.Errorf("CopyText = %q, %v", text, ok)
	}
	if _, ok := got[0].CopyText(); ok {
		t.Error("x.a is not a copy action")
	}
	if string(got[2].Args) != `{"n":1}` {
		t.Errorf("args = %s", got[2].Args)
	}
	out := RenderItem(items[0], DefaultTheme(), Options{Width: 40})
	for _, want := range []string{"[1] A", "[c] B", "[3] C"} {
		if !strings.Contains(out, want) {
			t.Errorf("output lacks %q:\n%s", want, out)
		}
	}
}

func TestItemHeaders(t *testing.T) {
	text := func(s string) protocol.Component {
		return protocol.Component{Value: protocol.TextComponent{Blocks: []protocol.Block{
			{Value: protocol.ParagraphBlock{Spans: []protocol.Span{{Text: s}}}},
		}}}
	}
	label := "photo-editor.crop"
	guest := "claude"
	tool := func(state protocol.ItemState) protocol.ViewItem {
		return protocol.ViewItem{Ref: "t", Kind: protocol.ItemKindTool, Label: &label, State: &state, Component: text("body")}
	}
	tests := []struct {
		item protocol.ViewItem
		opts Options
		want string
	}{
		{tool(protocol.ItemStatePending), Options{}, "○ photo-editor.crop\n  body"},
		{tool(protocol.ItemStateRunning), Options{}, "◐ photo-editor.crop\n  body"},
		{tool(protocol.ItemStateRunning), Options{Frame: 1}, "◓ photo-editor.crop\n  body"},
		{tool(protocol.ItemStateDone), Options{}, "✓ photo-editor.crop\n  body"},
		{tool(protocol.ItemStateFailed), Options{}, "✗ photo-editor.crop\n  body"},
		{protocol.ViewItem{Kind: protocol.ItemKindUser, Label: &label, Component: text("hi")}, Options{}, "» photo-editor.crop\nhi"},
		{protocol.ViewItem{Kind: protocol.ItemKindAgent, Component: text("hi")}, Options{}, "hi"},
		{protocol.ViewItem{Kind: protocol.ItemKindThought, Component: text("secret")}, Options{}, "… thinking"},
		{protocol.ViewItem{Kind: protocol.ItemKindThought, Component: text("secret")}, Options{Thoughts: true}, "secret"},
		{protocol.ViewItem{Kind: protocol.ItemKindAgent, Label: &guest, Component: text("hi")}, Options{}, "claude\nhi"},
		{protocol.ViewItem{Kind: protocol.ItemKindThought, Label: &guest, Component: text("secret")}, Options{}, "… claude thinking"},
		{protocol.ViewItem{Kind: protocol.ItemKindThought, Label: &guest, Component: text("secret")}, Options{Thoughts: true}, "secret"},
		{protocol.ViewItem{Kind: protocol.ItemKindNotice, Component: text("note")}, Options{}, "note"},
	}
	for _, tc := range tests {
		if got := RenderItem(tc.item, DefaultTheme(), tc.opts); got != tc.want {
			t.Errorf("%s/%v: got %q, want %q", tc.item.Kind, tc.item.State, got, tc.want)
		}
	}

	// Noticing errors: a failed notice is danger-colored, others muted.
	n := protocol.ViewItem{Kind: protocol.ItemKindNotice, Component: text("boom")}
	got := RenderItem(n, DefaultTheme(), Options{Color: TrueColor})
	if !strings.Contains(got, "38;2;138;143;156") { // muted
		t.Errorf("notice not muted: %q", got)
	}
}

func TestUnknownComponentsAreSkipped(t *testing.T) {
	var items []protocol.ViewItem
	data := `[{"ref":"r","kind":"agent","component":{"type":"stack","children":[
		{"type":"hologram","depth":3},
		{"type":"link","url":"https://x.y"}]}}]`
	if err := json.Unmarshal([]byte(data), &items); err != nil {
		t.Fatal(err)
	}
	if got := Render(items, DefaultTheme(), Options{}); got != "https://x.y\n" {
		t.Errorf("got %q", got)
	}
}

func TestEveryLineFitsTheWidth(t *testing.T) {
	for _, name := range fixtureNames(t) {
		items := loadFixture(t, name)
		for _, w := range []int{10, 12, 20, 25, 33, 40, 80, 120} {
			for _, l := range strings.Split(Render(items, DefaultTheme(), Options{Width: w}), "\n") {
				if Width(l) > w {
					t.Errorf("%s at %d: %q is %d wide", name, w, l, Width(l))
				}
			}
		}
	}
}

func para(text string) protocol.Block {
	return protocol.Block{Value: protocol.ParagraphBlock{Spans: []protocol.Span{{Text: text}}}}
}

func renderBlocks(t *testing.T, width int, blocks ...protocol.Block) string {
	t.Helper()
	return render(t, protocol.TextComponent{Blocks: blocks}, width)
}

func TestListMarkers(t *testing.T) {
	yes, no := true, false
	start := int64(9)
	ordered := protocol.ListBlock{Ordered: true, Start: &start, Items: []protocol.ListItem{
		{Blocks: []protocol.Block{para("nine")}},
		{Blocks: []protocol.Block{para("ten, long enough to wrap onto a second line")}},
	}}
	tasks := protocol.ListBlock{Items: []protocol.ListItem{
		{Checked: &yes, Blocks: []protocol.Block{para("done")}},
		{Checked: &no, Blocks: []protocol.Block{para("todo"), {Value: protocol.ListBlock{Items: []protocol.ListItem{
			{Blocks: []protocol.Block{para("nested")}},
		}}}}},
	}}
	got := renderBlocks(t, 24, protocol.Block{Value: ordered}, protocol.Block{Value: tasks})
	want := strings.Join([]string{
		" 9. nine",
		"10. ten, long enough to",
		"    wrap onto a second",
		"    line",
		"",
		"[x] done",
		"[ ] todo",
		"    • nested",
	}, "\n")
	if got != want {
		t.Errorf("lists:\n%s\nwant:\n%s", got, want)
	}
}

func TestQuoteKeepsItsBarThroughBlankLines(t *testing.T) {
	q := protocol.QuoteBlock{Blocks: []protocol.Block{para("first"), para("second")}}
	if got, want := renderBlocks(t, 40, protocol.Block{Value: q}), "│ first\n│\n│ second"; got != want {
		t.Errorf("quote:\n%s\nwant:\n%s", got, want)
	}
}

func TestCodeHardWrapsAtTheWidth(t *testing.T) {
	got := render(t, protocol.CodeComponent{Code: "0123456789abcdef\n\n\tx"}, 14)
	want := "  0123456789ab\n  cdef\n\n      x"
	if got != want {
		t.Errorf("code:\n%q\nwant:\n%q", got, want)
	}
}

func TestDiffLines(t *testing.T) {
	path := "a.go"
	one := int64(1)
	d := protocol.DiffComponent{Path: &path, Lines: []protocol.DiffLine{
		{Kind: protocol.DiffLineKindHunk, Text: "@@ -1 +1 @@"},
		{Kind: protocol.DiffLineKindRemove, Text: "old", Old: &one},
		{Kind: protocol.DiffLineKindAdd, Text: "new, and a long tail", New: &one},
		{Kind: protocol.DiffLineKindContext, Text: "same"},
	}}
	got := render(t, d, 12)
	want := "a.go\n@@ -1 +1 @@\n-old\n+new, and a\n long tail\n same"
	if got != want {
		t.Errorf("diff:\n%q\nwant:\n%q", got, want)
	}
}

func TestImageAndFileFallbacks(t *testing.T) {
	w, h := int64(640), int64(480)
	caption := "A caption"
	name := "logo"
	tests := []struct {
		c    protocol.ComponentVariant
		want string
	}{
		{protocol.ImageComponent{Src: "asset://a", Width: &w, Height: &h, Caption: &caption}, "[image 640×480 · asset://a]\nA caption"},
		{protocol.ImageComponent{Src: "asset://a", Width: &w}, "[image asset://a]"}, // size needs both
		{protocol.FileComponent{Path: "/a/b.txt"}, "📄 /a/b.txt"},
		{protocol.FileComponent{Path: "/a/b.txt", Name: &name}, "📄 logo (/a/b.txt)"},
	}
	for _, tc := range tests {
		if got := render(t, tc.c, 60); got != tc.want {
			t.Errorf("%T = %q, want %q", tc.c, got, tc.want)
		}
	}
}

func TestTextRoleAndToneStyles(t *testing.T) {
	title, success := protocol.TextRoleTitle, protocol.ToneSuccess
	item := protocol.ViewItem{Kind: protocol.ItemKindAgent, Component: protocol.Component{Value: protocol.TextComponent{
		Role: &title, Tone: &success, Blocks: []protocol.Block{para("Done")},
	}}}
	got := RenderItem(item, DefaultTheme(), Options{Color: TrueColor})
	if want := "\x1b[1;38;2;158;206;106mDone\x1b[0m"; got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestUntrustedTextCannotInjectEscapes(t *testing.T) {
	evil := "x\x1b[2J\x1b]52;c;AAAA\x07y"
	label := evil
	tbl := protocol.TableComponent{Columns: []string{evil}, Rows: [][]string{{evil}}}
	kv := protocol.KvComponent{Items: []protocol.KvItem{{Key: evil, Value: evil}}}
	state := protocol.ItemStateDone
	for _, c := range []protocol.ComponentVariant{
		protocol.CodeComponent{Code: evil},
		protocol.TextComponent{Blocks: []protocol.Block{para(evil)}},
		protocol.FileComponent{Path: evil},
		protocol.ImageComponent{Src: evil, Caption: &evil},
		protocol.ButtonComponent{Label: evil, Action: "a.b", Key: &evil},
		protocol.LinkComponent{URL: evil, Text: &evil},
		protocol.CardComponent{Title: &evil},
		protocol.DiffComponent{Path: &evil, Lines: []protocol.DiffLine{{Kind: protocol.DiffLineKindAdd, Text: evil}}},
		tbl, kv,
	} {
		item := protocol.ViewItem{Kind: protocol.ItemKindTool, Label: &label, State: &state, Component: protocol.Component{Value: c}}
		if out := RenderItem(item, DefaultTheme(), Options{}); strings.ContainsAny(out, "\x1b\x07") {
			t.Errorf("%T: escape in output %q", c, out)
		}
	}
}
