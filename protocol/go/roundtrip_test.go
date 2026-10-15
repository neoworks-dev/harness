package protocol

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// Every example in protocol/examples decodes into a known Go variant and re-encodes to the same JSON.
func TestExamplesRoundTrip(t *testing.T) {
	files, err := filepath.Glob("../examples/*.json")
	if err != nil || len(files) == 0 {
		t.Fatalf("no examples found: %v", err)
	}
	for _, file := range files {
		name := strings.TrimSuffix(filepath.Base(file), ".json")
		t.Run(name, func(t *testing.T) {
			original, err := os.ReadFile(file)
			if err != nil {
				t.Fatal(err)
			}
			var env Envelope
			if err := json.Unmarshal(original, &env); err != nil {
				t.Fatalf("decode: %v", err)
			}
			if u, ok := env.Value.(UnknownEnvelope); ok {
				t.Fatalf("topic %q decoded as unknown", u.Tag)
			}
			if ev, ok := env.Value.(SessionEvents); ok {
				if u, ok := ev.Payload.Event.Value.(UnknownSessionEvent); ok {
					t.Fatalf("event type %q decoded as unknown", u.Tag)
				}
			}
			encoded, err := json.Marshal(env)
			if err != nil {
				t.Fatalf("encode: %v", err)
			}
			assertSameJSON(t, original, encoded)
		})
	}
}

func TestUnknownVariantsSurviveRoundTrip(t *testing.T) {
	in := []byte(`{"topic":"session.events","payload":{"session":"s","seq":3,"ts":1,"event":{"type":"from_the_future","x":[1,2]}}}`)
	var env Envelope
	if err := json.Unmarshal(in, &env); err != nil {
		t.Fatal(err)
	}
	ev := env.Value.(SessionEvents).Payload.Event.Value
	if u, ok := ev.(UnknownSessionEvent); !ok || u.Tag != "from_the_future" {
		t.Fatalf("got %#v", ev)
	}
	out, err := json.Marshal(env)
	if err != nil {
		t.Fatal(err)
	}
	assertSameJSON(t, in, out)
}

func TestBuildEnvelopeFromGo(t *testing.T) {
	text := "hi"
	out, err := json.Marshal(Envelope{Value: SessionInputMessage{ID: &text, Payload: SessionInput{Session: "s", App: "cli", Text: "hello"}}})
	if err != nil {
		t.Fatal(err)
	}
	assertSameJSON(t, []byte(`{"topic":"session.input","id":"hi","payload":{"session":"s","app":"cli","text":"hello"}}`), out)
}

func TestMissingTagIsAnError(t *testing.T) {
	var env Envelope
	if err := json.Unmarshal([]byte(`{"payload":{}}`), &env); err == nil {
		t.Fatal("expected error")
	}
}

func assertSameJSON(t *testing.T, want, got []byte) {
	t.Helper()
	var a, b any
	if err := json.Unmarshal(want, &a); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(got, &b); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(a, b) {
		t.Fatalf("JSON differs\nwant: %s\n got: %s", want, got)
	}
}
