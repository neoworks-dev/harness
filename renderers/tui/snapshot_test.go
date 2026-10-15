package tui

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	protocol "neoworks.dev/suite/protocol/go"
)

const fixturesDir = "../../fixtures"

func loadFixture(t *testing.T, name string) []protocol.ViewItem {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(fixturesDir, name, "resolved.json"))
	if err != nil {
		t.Fatal(err)
	}
	var items []protocol.ViewItem
	if err := json.Unmarshal(data, &items); err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	return items
}

func fixtureNames(t *testing.T) []string {
	t.Helper()
	paths, err := filepath.Glob(filepath.Join(fixturesDir, "*", "resolved.json"))
	if err != nil || len(paths) == 0 {
		t.Fatalf("no fixtures found in %s: %v", fixturesDir, err)
	}
	names := make([]string, len(paths))
	for i, p := range paths {
		names[i] = filepath.Base(filepath.Dir(p))
	}
	return names
}

// compareSnapshot checks got against the file, or rewrites the file when
// UPDATE_SNAPSHOTS is set.
func compareSnapshot(t *testing.T, file, got string) {
	t.Helper()
	if os.Getenv("UPDATE_SNAPSHOTS") != "" {
		if err := os.WriteFile(file, []byte(got), 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	want, err := os.ReadFile(file)
	if err != nil {
		t.Fatalf("%v (run with UPDATE_SNAPSHOTS=1 to create it)", err)
	}
	if got != string(want) {
		t.Errorf("%s differs (UPDATE_SNAPSHOTS=1 rewrites it)\n--- got ---\n%s\n--- want ---\n%s", file, got, want)
	}
}

// Every fixture is drawn at 80 and 40 columns without color and compared with
// tui.txt and tui-40.txt next to it. The other renderers draw the same files.
func TestSnapshots(t *testing.T) {
	theme := DefaultTheme()
	for _, name := range fixtureNames(t) {
		items := loadFixture(t, name)
		for _, c := range []struct {
			file  string
			width int
		}{{"tui.txt", 80}, {"tui-40.txt", 40}} {
			t.Run(name+"/"+c.file, func(t *testing.T) {
				got := Render(items, theme, Options{Width: c.width})
				compareSnapshot(t, filepath.Join(fixturesDir, name, c.file), got)
			})
		}
	}
}

// One fixture with escape codes, so color regressions show up in review.
func TestSnapshotANSI256(t *testing.T) {
	items := loadFixture(t, "chat-turn")
	got := Render(items, DefaultTheme(), Options{Width: 80, Color: ANSI256})
	compareSnapshot(t, filepath.Join(fixturesDir, "chat-turn", "tui-256.ans"), got)
}
