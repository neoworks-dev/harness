package main

import (
	"strings"
	"testing"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
)

func TestSocketPath(t *testing.T) {
	env := func(m map[string]string) func(string) string { return func(k string) string { return m[k] } }
	tests := []struct {
		name string
		flag string
		env  map[string]string
		want string
	}{
		{"flag wins", "/f.sock", map[string]string{"SUITE_SOCKET": "/e.sock", "XDG_RUNTIME_DIR": "/run/user/1"}, "/f.sock"},
		{"env over runtime dir", "", map[string]string{"SUITE_SOCKET": "/e.sock", "XDG_RUNTIME_DIR": "/run/user/1"}, "/e.sock"},
		{"runtime dir", "", map[string]string{"XDG_RUNTIME_DIR": "/run/user/1"}, "/run/user/1/suite/bus.sock"},
		{"tmp fallback", "", nil, "/tmp/suite-1000/bus.sock"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := socketPath(tc.flag, env(tc.env), 1000); got != tc.want {
				t.Errorf("got %q, want %q", got, tc.want)
			}
		})
	}
}

func TestResolveSession(t *testing.T) {
	sessions := []protocol.SessionInfo{{ID: "abc123"}, {ID: "abd456"}, {ID: "xyz"}, {ID: "xyz2"}}
	tests := []struct {
		prefix  string
		want    string
		wantErr string
	}{
		{"abc", "abc123", ""},
		{"abc123", "abc123", ""},
		{"x", "", "ambiguous"},
		{"ab", "", "ambiguous"},
		{"xyz", "xyz", ""}, // an exact id beats longer ids sharing the prefix
		{"q", "", "no session matches"},
		{"", "", "no session matches"},
	}
	for _, tc := range tests {
		t.Run(tc.prefix, func(t *testing.T) {
			got, err := resolveSession(sessions, tc.prefix)
			if tc.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), tc.wantErr) {
					t.Fatalf("err = %v, want containing %q", err, tc.wantErr)
				}
				return
			}
			if err != nil || got.ID != tc.want {
				t.Errorf("got %q, %v; want %q", got.ID, err, tc.want)
			}
		})
	}
}

func TestRelativeTime(t *testing.T) {
	now := time.Date(2026, 1, 10, 12, 0, 0, 0, time.UTC)
	tests := []struct {
		ago  time.Duration
		want string
	}{
		{time.Second, "just now"},
		{30 * time.Second, "30s ago"},
		{2 * time.Minute, "2m ago"},
		{3 * time.Hour, "3h ago"},
		{49 * time.Hour, "2d ago"},
	}
	for _, tc := range tests {
		if got := relativeTime(now, now.Add(-tc.ago)); got != tc.want {
			t.Errorf("%v: got %q, want %q", tc.ago, got, tc.want)
		}
	}
}
