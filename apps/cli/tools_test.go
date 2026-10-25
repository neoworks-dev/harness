package main

import (
	"strings"
	"testing"

	protocol "neoworks.dev/neo/protocol/go"
	"neoworks.dev/neo/sdk-go/sdktest"
)

func seedConnectors(d *sdktest.Daemon) {
	d.SetConnector(protocol.ConnectorInfo{
		App: "photo", Kind: protocol.ConnectorKindApp, Connected: true, V: 3,
		Tools: []protocol.ConnectorTool{
			{Name: "photo.crop", Description: "Crop an image to a rectangle"},
			{Name: "photo.blur", Description: "Apply a Gaussian\nblur to the selection"},
		},
	})
	d.SetConnector(protocol.ConnectorInfo{
		App: "browser", Kind: protocol.ConnectorKindApp, V: 1, Error: ptr("connection refused"),
		Tools: []protocol.ConnectorTool{{Name: "browser.open", Description: "Open a URL and crop nothing"}},
	})
	d.SetConnector(protocol.ConnectorInfo{App: "lsp", Kind: protocol.ConnectorKindService, Connected: true, V: 2})
}

func TestToolsListsConnectors(t *testing.T) {
	d := sdktest.New(t)
	seedConnectors(d)

	code, stdout, stderr := exec(t, d.Socket, "", "tools")
	if code != 0 || stderr != "" {
		t.Fatalf("code %d, stderr %q", code, stderr)
	}
	want := "browser (app, closed, v1) error: connection refused\n" +
		"  browser.open — Open a URL and crop nothing\n" +
		"lsp (service, connected, v2)\n" +
		"  (no tools)\n" +
		"photo (app, connected, v3)\n" +
		"  photo.crop — Crop an image to a rectangle\n" +
		"  photo.blur — Apply a Gaussian blur to the selection\n"
	if stdout != want {
		t.Errorf("stdout:\n%s\nwant:\n%s", stdout, want)
	}
}

func TestToolsQuery(t *testing.T) {
	d := sdktest.New(t)
	seedConnectors(d)

	tests := []struct {
		name string
		args []string
		want string
	}{
		{
			"words match name or description, case-insensitively, grouped by app",
			[]string{"tools", "CROP"},
			"browser (app, closed, v1) error: connection refused\n  browser.open — Open a URL and crop nothing\n" +
				"photo (app, connected, v3)\n  photo.crop — Crop an image to a rectangle\n",
		},
		{
			"all words must match",
			[]string{"tools", "crop", "image"},
			"photo (app, connected, v3)\n  photo.crop — Crop an image to a rectangle\n",
		},
		{
			"query may be one quoted argument",
			[]string{"tools", "gaussian blur"},
			"photo (app, connected, v3)\n  photo.blur — Apply a Gaussian blur to the selection\n",
		},
		{"no match", []string{"tools", "nothing", "gaussian"}, "no tools match \"nothing gaussian\"\n"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			code, stdout, stderr := exec(t, d.Socket, "", tc.args...)
			if code != 0 || stderr != "" || stdout != tc.want {
				t.Errorf("code %d stderr %q stdout:\n%s\nwant:\n%s", code, stderr, stdout, tc.want)
			}
		})
	}
}

func TestToolsNoConnectors(t *testing.T) {
	d := sdktest.New(t)
	for _, args := range [][]string{{"tools"}, {"tools", "crop"}} {
		code, stdout, stderr := exec(t, d.Socket, "", args...)
		if code != 0 || stderr != "" || stdout != "no connectors\n" {
			t.Errorf("%v: code %d stdout %q stderr %q", args, code, stdout, stderr)
		}
	}
}

func TestToolLineFitsWidth(t *testing.T) {
	tool := protocol.ConnectorTool{Name: "a.b", Description: "one two three four five six seven"}
	tests := []struct {
		width int
		want  string
	}{
		{100, "  a.b — one two three four five six seven"},
		{20, "  a.b — one two thr…"},
		{8, "  a.b"},
	}
	for _, tc := range tests {
		got := toolLine(tool, tc.width)
		if got != tc.want || len([]rune(got)) > max(tc.width, len("  a.b")) {
			t.Errorf("width %d: got %q, want %q", tc.width, got, tc.want)
		}
	}
	if got := toolLine(protocol.ConnectorTool{Name: "x.y"}, 80); got != "  x.y" {
		t.Errorf("no description: %q", got)
	}
	if !strings.HasSuffix(toolLine(protocol.ConnectorTool{Name: "x", Description: strings.Repeat("é", 200)}, 50), "…") {
		t.Error("long multibyte description not cut")
	}
}
