package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
	"neoworks.dev/suite/renderers/tui"
	sdk "neoworks.dev/suite/sdk-go"
	"neoworks.dev/suite/sdk-go/sdktest"
)

type syncBuf struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (s *syncBuf) Write(p []byte) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.Write(p)
}

func (s *syncBuf) String() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.String()
}

// The headless app against the real daemon: registers, and the gateway lists
// and runs its tools; results are assets in the daemon's store and render
// with the app's templates.
func TestHeadlessAgainstRealDaemon(t *testing.T) {
	d := sdktest.StartReal(t)
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()

	obs, err := sdk.Dial(ctx, sdk.Options{Socket: d.Socket, Client: "observer"})
	if err != nil {
		t.Fatal(err)
	}
	defer obs.Close()

	appCtx, stopApp := context.WithCancel(ctx)
	var stderr syncBuf
	done := make(chan error, 1)
	go func() {
		done <- run(appCtx, config{Socket: d.Socket, Headless: true, Stderr: &stderr, Launch: []string{"/usr/bin/photo"}})
	}()
	stopped := false
	defer func() {
		if !stopped {
			stopApp()
			<-done
		}
	}()

	var info protocol.ConnectorInfo
	sdktest.Eventually(t, "the gateway to list all nine tools", func() bool {
		list, err := obs.ListConnectors(ctx)
		if err != nil {
			return false
		}
		for _, c := range list {
			if c.App == "photo-editor" && c.Connected && len(c.Tools) == 9 && c.Error == nil {
				info = c
				return true
			}
		}
		return false
	})
	if info.Kind != protocol.ConnectorKindApp || info.Title == nil || *info.Title != "Photo editor" {
		t.Errorf("connector %+v", info)
	}

	// The logo flow's middle step: an asset from another app is opened, cropped.
	src := writePNG(t, t.TempDir(), "download.png", 200, 120)
	data, err := os.ReadFile(src)
	if err != nil {
		t.Fatal(err)
	}
	enc := base64.StdEncoding.EncodeToString(data)
	name := "download.png"
	downloaded, err := obs.PutAsset(ctx, protocol.AssetPut{Data: &enc, Name: &name})
	if err != nil {
		t.Fatal(err)
	}

	call := func(tool string, args map[string]any) sdktest.ToolResult {
		t.Helper()
		res, err := d.CallTool(ctx, "photo-editor."+tool, args)
		if err != nil {
			t.Fatalf("%s: %v", tool, err)
		}
		if res.IsError {
			t.Fatalf("%s failed: %s", tool, res.Text())
		}
		return res
	}
	handleOf := func(res sdktest.ToolResult) string {
		h, _ := res.StructuredContent["handle"].(string)
		if !strings.HasPrefix(h, "asset://img-") {
			t.Fatalf("no handle in %+v", res.StructuredContent)
		}
		return h
	}

	opened := call("open", map[string]any{"asset": downloaded.Handle})
	if handleOf(opened) != downloaded.Handle || opened.StructuredContent["width"] != 200.0 {
		t.Errorf("open: %+v", opened.StructuredContent)
	}

	cropped := call("crop", map[string]any{"x": 20, "y": 10, "width": 100, "height": 60})
	handle := handleOf(cropped)
	if handle == downloaded.Handle || cropped.StructuredContent["width"] != 100.0 || cropped.StructuredContent["height"] != 60.0 || cropped.StructuredContent["mime"] != "image/png" {
		t.Fatalf("crop: %+v", cropped.StructuredContent)
	}
	if !strings.Contains(cropped.Text(), handle) {
		t.Errorf("text %q lacks the handle", cropped.Text())
	}

	// A new asset exists in the daemon's store, made by the photo editor, with the right size.
	asset, err := obs.GetAsset(ctx, handle)
	if err != nil {
		t.Fatal(err)
	}
	if asset.Mime != "image/png" || asset.App == nil || *asset.App != "photo-editor" {
		t.Errorf("asset %+v", asset)
	}
	if w, h := assetSize(t, asset.Path); w != 100 || h != 60 {
		t.Errorf("stored asset is %d×%d, want 100×60", w, h)
	}

	// The presence context follows the open image.
	sdktest.Eventually(t, "the context to name the cropped image", func() bool {
		list, _ := obs.ListConnectors(ctx)
		for _, c := range list {
			if c.App == "photo-editor" && c.Context["asset"] == handle && c.Context["image"] == "download-crop.png" {
				return true
			}
		}
		return false
	})

	// The gateway validates against the input schema before the app sees the call.
	if res, err := d.CallTool(ctx, "photo-editor.crop", map[string]any{"x": 0}); err != nil || !res.IsError || !strings.Contains(res.Text(), "Invalid arguments") {
		t.Errorf("invalid call: %v %+v", err, res)
	}
	// Errors from the editor itself come back as tool errors the model can read.
	if res, err := d.CallTool(ctx, "photo-editor.crop", map[string]any{"x": 90, "y": 0, "width": 50, "height": 10}); err != nil || !res.IsError || !strings.Contains(res.Text(), "100×60") {
		t.Errorf("out of bounds: %v %+v", err, res)
	}

	// Undo goes back to the opened asset without a new one.
	if got := handleOf(call("undo", nil)); got != downloaded.Handle {
		t.Errorf("undo gave %s, want %s", got, downloaded.Handle)
	}

	// A button press runs the tool and the result renders with the app's template.
	sess, err := obs.CreateSession(ctx, protocol.SessionCreateParams{})
	if err != nil {
		t.Fatal(err)
	}
	view, err := obs.SubscribeView(ctx, sess.ID)
	if err != nil {
		t.Fatal(err)
	}
	defer view.Close()
	if err := obs.Action(ctx, sess.ID, "none", "photo-editor.rotate", []byte(`{"degrees":90}`)); err != nil {
		t.Fatal(err)
	}
	state := view.State()
	var card protocol.ViewItem
	deadline := time.After(10 * time.Second)
	for card.Ref == "" {
		select {
		case u, ok := <-view.Updates():
			if !ok {
				t.Fatalf("view ended: %v", view.Err())
			}
			if err := u.ApplyTo(state); err != nil {
				t.Fatal(err)
			}
			for _, it := range state.Items() {
				if it.Kind == protocol.ItemKindTool && it.State != nil && *it.State == protocol.ItemStateDone {
					card = it
				}
			}
		case <-deadline:
			t.Fatalf("no finished tool item; items: %+v", state.Items())
		}
	}
	text := tui.RenderItem(card, tui.DefaultTheme(), tui.Options{Width: 80})
	for _, want := range []string{"Rotated image", "[image 120×200", "download-rotate.png", "[u] Undo", "[c] Copy handle"} {
		if !strings.Contains(text, want) {
			t.Errorf("template output lacks %q:\n%s", want, text)
		}
	}
	var buttons []string
	for _, b := range tui.Buttons(card) {
		buttons = append(buttons, b.Key+"="+b.Action)
	}
	if len(buttons) != 3 || !strings.HasPrefix(buttons[2], "c=copy:asset://img-") {
		t.Errorf("buttons %v", buttons)
	}

	// Shutting down unregisters: the connector shows as closed.
	stopApp()
	stopped = true
	select {
	case err := <-done:
		if err != nil {
			t.Errorf("run: %v", err)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("the app did not stop")
	}
	sdktest.Eventually(t, "the connector to close", func() bool {
		list, _ := obs.ListConnectors(ctx)
		for _, c := range list {
			if c.App == "photo-editor" {
				return !c.Connected
			}
		}
		return false
	})
	res, err := d.CallTool(ctx, "photo-editor.info", nil)
	if err != nil || !res.IsError || !strings.Contains(res.Text(), "isn't open") || !strings.Contains(res.Text(), "apps_launch") {
		t.Errorf("closed app: %v %+v", err, res)
	}
	if got := stderr.String(); !strings.Contains(got, "registered as photo-editor") {
		t.Errorf("headless log: %q", got)
	}
}
