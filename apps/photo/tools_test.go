package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"neoworks.dev/suite/sdk-go/tools"
)

// session drives the tools like the gateway would.
type session struct {
	t     *testing.T
	store *memAssets
	list  []tools.Tool
}

func newSession(t *testing.T) *session {
	store := newMemAssets(t)
	return &session{t: t, store: store, list: photoTools(newEditor(store))}
}

// call runs a tool and returns its structured result (or the error text).
func (s *session) call(name string, args map[string]any) (imageResult, error) {
	s.t.Helper()
	// Arguments arrive decoded from JSON, so numbers are float64.
	raw, err := json.Marshal(args)
	if err != nil {
		s.t.Fatal(err)
	}
	decoded := map[string]any{}
	if err := json.Unmarshal(raw, &decoded); err != nil {
		s.t.Fatal(err)
	}
	res, err := toolByName(s.t, s.list, name).Handler(context.Background(), decoded)
	if err != nil {
		return imageResult{}, err
	}
	data, err := json.Marshal(res.Structured)
	if err != nil {
		s.t.Fatal(err)
	}
	var out imageResult
	if err := json.Unmarshal(data, &out); err != nil {
		s.t.Fatal(err)
	}
	if !strings.Contains(res.Text, out.Handle) {
		s.t.Errorf("%s: the text %q lacks the handle", name, res.Text)
	}
	return out, nil
}

func (s *session) must(name string, args map[string]any) imageResult {
	s.t.Helper()
	res, err := s.call(name, args)
	if err != nil {
		s.t.Fatalf("%s: %v", name, err)
	}
	if res.Handle == "" || res.Name == "" || res.Mime == "" {
		s.t.Fatalf("%s: incomplete result %+v", name, res)
	}
	return res
}

func (s *session) size(res imageResult) (int, int) {
	s.t.Helper()
	a, err := s.store.GetAsset(context.Background(), res.Handle)
	if err != nil {
		s.t.Fatal(err)
	}
	return assetSize(s.t, a.Path)
}

func TestEditingFlow(t *testing.T) {
	s := newSession(t)
	path := writePNG(t, t.TempDir(), "logo.png", 200, 100)

	opened := s.must("open", map[string]any{"path": path})
	if opened.Width != 200 || opened.Height != 100 || opened.Mime != "image/png" || opened.Name != "logo.png" {
		t.Fatalf("open: %+v", opened)
	}
	if info := s.must("info", nil); info != opened {
		t.Errorf("info %+v != open %+v", info, opened)
	}

	cropped := s.must("crop", map[string]any{"x": 10, "y": 20, "width": 80, "height": 40})
	if cropped.Width != 80 || cropped.Height != 40 || cropped.Handle == opened.Handle {
		t.Fatalf("crop: %+v", cropped)
	}
	if w, h := s.size(cropped); w != 80 || h != 40 {
		t.Errorf("stored crop is %d×%d", w, h)
	}

	resized := s.must("resize", map[string]any{"width": 40})
	if resized.Width != 40 || resized.Height != 20 {
		t.Errorf("resize keeps aspect: %+v", resized)
	}
	rotated := s.must("rotate", map[string]any{"degrees": 90})
	if rotated.Width != 20 || rotated.Height != 40 {
		t.Errorf("rotate: %+v", rotated)
	}
	flipped := s.must("flip", map[string]any{"direction": "vertical"})
	gray := s.must("grayscale", nil)
	if gray.Width != 20 || gray.Height != 40 || gray.Handle == flipped.Handle {
		t.Errorf("grayscale: %+v", gray)
	}

	// Export leaves the current image alone.
	jpg := s.must("export", map[string]any{"format": "jpeg", "quality": 70})
	if jpg.Mime != "image/jpeg" || !strings.HasSuffix(jpg.Name, ".jpg") || jpg.Width != 20 || jpg.Handle == gray.Handle {
		t.Errorf("export jpeg: %+v", jpg)
	}
	if png := s.must("export", map[string]any{"format": "png"}); png.Mime != "image/png" || png.Handle != gray.Handle {
		// the same pixels encoded the same way give the same content-addressed asset
		t.Errorf("export png: %+v (current %+v)", png, gray)
	}
	if info := s.must("info", nil); info.Handle != gray.Handle {
		t.Errorf("export changed the current image: %+v", info)
	}

	// Undo walks back through the handles that were handed out, creating nothing.
	puts := s.store.puts
	for _, want := range []imageResult{flipped, rotated, resized, cropped, opened} {
		if got := s.must("undo", nil); got != want {
			t.Errorf("undo: got %+v, want %+v", got, want)
		}
	}
	if s.store.puts != puts {
		t.Error("undo stored an asset")
	}
	if _, err := s.call("undo", nil); err == nil || !strings.Contains(err.Error(), "nothing to undo") {
		t.Errorf("undo past the start: %v", err)
	}
	// Earlier handles stay valid: open one of them again.
	again := s.must("open", map[string]any{"asset": cropped.Handle})
	if again.Width != 80 || again.Height != 40 || again.Handle != cropped.Handle {
		t.Errorf("reopen: %+v", again)
	}
}

func TestOpenAssetAndErrors(t *testing.T) {
	s := newSession(t)
	if _, err := s.call("crop", map[string]any{"x": 0, "y": 0, "width": 1, "height": 1}); err == nil || !strings.Contains(err.Error(), "open") {
		t.Errorf("crop before open: %v", err)
	}
	for _, name := range []string{"info", "undo", "grayscale"} {
		if _, err := s.call(name, nil); err == nil {
			t.Errorf("%s without an image succeeded", name)
		}
	}
	if _, err := s.call("open", nil); err == nil {
		t.Error("open with no argument succeeded")
	}
	if _, err := s.call("open", map[string]any{"asset": "asset://img-1", "path": "/x"}); err == nil {
		t.Error("open with both arguments succeeded")
	}
	if _, err := s.call("open", map[string]any{"asset": "asset://img-000000000000"}); err == nil {
		t.Error("open of an unknown asset succeeded")
	}
	if _, err := s.call("open", map[string]any{"path": "/does/not/exist.png"}); err == nil {
		t.Error("open of a missing file succeeded")
	}

	// An asset that is not an image.
	txt := "hello"
	a, err := s.store.PutAsset(context.Background(), putData(txt, "note.txt", "text/plain"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.call("open", map[string]any{"asset": a.Handle}); err == nil || !strings.Contains(err.Error(), "not an image") {
		t.Errorf("open of text: %v", err)
	}

	// Opening an asset made elsewhere works.
	path := writePNG(t, t.TempDir(), "x.png", 30, 20)
	other, err := s.store.PutAsset(context.Background(), putPath(path))
	if err != nil {
		t.Fatal(err)
	}
	if res := s.must("open", map[string]any{"asset": other.Handle}); res.Width != 30 || res.Handle != other.Handle {
		t.Errorf("%+v", res)
	}
	// A handle given as path is opened as the asset; relative paths are refused.
	if res := s.must("open", map[string]any{"path": other.Handle}); res.Handle != other.Handle {
		t.Errorf("handle as path: %+v", res)
	}
	if _, err := s.call("open", map[string]any{"path": "x.png"}); err == nil || !strings.Contains(err.Error(), "absolute") {
		t.Errorf("relative path: %v", err)
	}
}

func TestBadArguments(t *testing.T) {
	s := newSession(t)
	s.must("open", map[string]any{"path": writePNG(t, t.TempDir(), "p.png", 50, 40)})
	for name, tc := range map[string]struct {
		tool string
		args map[string]any
		want string
	}{
		"crop outside":     {"crop", map[string]any{"x": 40, "y": 0, "width": 20, "height": 10}, "50×40"},
		"crop missing":     {"crop", map[string]any{"x": 0, "y": 0, "width": 5}, "height is required"},
		"crop fractional":  {"crop", map[string]any{"x": 0.5, "y": 0, "width": 5, "height": 5}, "integer"},
		"crop string":      {"crop", map[string]any{"x": "0", "y": 0, "width": 5, "height": 5}, "integer"},
		"resize nothing":   {"resize", map[string]any{}, "width"},
		"resize negative":  {"resize", map[string]any{"width": -4}, "positive"},
		"resize huge":      {"resize", map[string]any{"width": 100000}, "exceeds"},
		"rotate 45":        {"rotate", map[string]any{"degrees": 45}, "90, 180 or 270"},
		"rotate missing":   {"rotate", map[string]any{}, "degrees is required"},
		"flip sideways":    {"flip", map[string]any{"direction": "sideways"}, "horizontal"},
		"export gif":       {"export", map[string]any{"format": "gif"}, "png"},
		"export quality":   {"export", map[string]any{"format": "jpeg", "quality": 0}, "between 1 and 100"},
		"export no format": {"export", map[string]any{}, "format"},
	} {
		_, err := s.call(tc.tool, tc.args)
		if err == nil || !strings.Contains(err.Error(), tc.want) {
			t.Errorf("%s: error %v, want it to mention %q", name, err, tc.want)
		}
	}
	// Failed edits leave the image and the history as they were.
	if info := s.must("info", nil); info.Width != 50 || info.Height != 40 {
		t.Errorf("%+v", info)
	}
	if _, err := s.call("undo", nil); err == nil {
		t.Error("a failed edit left something to undo")
	}
}

func TestOpenResetsHistory(t *testing.T) {
	s := newSession(t)
	dir := t.TempDir()
	s.must("open", map[string]any{"path": writePNG(t, dir, "a.png", 10, 10)})
	s.must("grayscale", nil)
	s.must("open", map[string]any{"path": writePNG(t, dir, "b.png", 12, 12)})
	if _, err := s.call("undo", nil); err == nil {
		t.Error("undo reached the previous image")
	}
}

func TestHistoryIsBounded(t *testing.T) {
	s := newSession(t)
	s.must("open", map[string]any{"path": writePNG(t, t.TempDir(), "a.png", 8, 8)})
	for range maxHistory + 5 {
		s.must("rotate", map[string]any{"degrees": 90})
	}
	undone := 0
	for {
		if _, err := s.call("undo", nil); err != nil {
			break
		}
		undone++
	}
	if undone != maxHistory-1 {
		t.Errorf("undid %d steps, want %d", undone, maxHistory-1)
	}
}

func TestSchemasAndTemplates(t *testing.T) {
	list := photoTools(newEditor(newMemAssets(t)))
	names := map[string]bool{}
	for _, tool := range list {
		names[tool.Name] = true
		if tool.Description == "" || tool.OutputSchema == nil {
			t.Errorf("%s lacks a description or output schema", tool.Name)
		}
	}
	for _, want := range []string{"open", "info", "crop", "resize", "rotate", "flip", "grayscale", "export", "undo"} {
		if !names[want] {
			t.Errorf("no tool %s", want)
		}
	}
	for name, raw := range templates() {
		if !names[name] {
			t.Errorf("template for unknown tool %s", name)
		}
		var card struct {
			Type     string `json:"type"`
			Title    string `json:"title"`
			Children []struct {
				Type     string            `json:"type"`
				Src      string            `json:"src"`
				Children []json.RawMessage `json:"children"`
			} `json:"children"`
		}
		if err := json.Unmarshal(raw, &card); err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if card.Type != "card" || card.Title == "" || len(card.Children) != 3 {
			t.Errorf("%s: %+v", name, card)
			continue
		}
		if card.Children[0].Type != "image" || card.Children[0].Src != "{{data.handle}}" {
			t.Errorf("%s: image %+v", name, card.Children[0])
		}
		if !strings.Contains(string(raw), `"copy:{{data.handle}}"`) {
			t.Errorf("%s: no copy button", name)
		}
	}
	for name := range names {
		if _, ok := templates()[name]; !ok {
			t.Errorf("no template for %s", name)
		}
	}
}

func TestPresenceContext(t *testing.T) {
	srv, err := tools.Start(tools.Options{Name: "x", Tools: nil})
	if err != nil {
		t.Fatal(err)
	}
	defer srv.Close()
	p := presence(srv, []string{"/bin/photo"}, nil)
	if p.App != "photo-editor" || !p.Open || p.Mcp == nil || p.Launch == nil || p.Launch.Command[0] != "/bin/photo" || len(p.Templates) == 0 || p.Context != nil {
		t.Errorf("%+v", p)
	}
	s := newSession(t)
	path := writePNG(t, t.TempDir(), "face.png", 5, 5)
	s.must("open", map[string]any{"path": path})
	ed := newEditor(s.store)
	if _, err := ed.openPath(context.Background(), path); err != nil {
		t.Fatal(err)
	}
	d, _ := ed.current()
	p = presence(srv, nil, d)
	if p.Context["image"] != "face.png" || p.Context["asset"] != d.asset.Handle || p.Launch != nil {
		t.Errorf("%+v", p.Context)
	}
}
