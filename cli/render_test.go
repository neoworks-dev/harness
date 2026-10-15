package main

import (
	"strings"
	"testing"

	protocol "neoworks.dev/suite/protocol/go"
)

func ptr[T any](v T) *T { return &v }

func TestRender(t *testing.T) {
	long := strings.TrimSuffix(strings.Repeat("line\n", 13), "\n")
	tests := []struct {
		name     string
		thoughts bool
		events   []protocol.SessionEventVariant
		want     string
	}{
		{
			name:   "user message",
			events: []protocol.SessionEventVariant{protocol.UserMessage{App: "overlay", Text: "hi"}},
			want:   "\n» overlay: hi\n\n",
		},
		{
			name:   "user message with images",
			events: []protocol.SessionEventVariant{protocol.UserMessage{App: "overlay", Text: "look", Images: []string{"/tmp/a.png", "/tmp/b b.jpg"}}},
			want:   "\n» overlay: look\n  [image: /tmp/a.png]\n  [image: /tmp/b b.jpg]\n\n",
		},
		{
			name: "user message after unfinished line starts on a fresh line",
			events: []protocol.SessionEventVariant{
				protocol.AgentMessageChunk{MessageID: "m", Text: "partial"},
				protocol.UserMessage{App: "cli", Text: "hi"},
			},
			want: "partial\n\n» cli: hi\n\n",
		},
		{
			name: "chunks of one message join up",
			events: []protocol.SessionEventVariant{
				protocol.AgentMessageChunk{MessageID: "m1", Text: "Hel"},
				protocol.AgentMessageChunk{MessageID: "m1", Text: "lo"},
			},
			want: "Hello",
		},
		{
			name: "new messageId starts a fresh line",
			events: []protocol.SessionEventVariant{
				protocol.AgentMessageChunk{MessageID: "m1", Text: "one"},
				protocol.AgentMessageChunk{MessageID: "m2", Text: "two"},
			},
			want: "one\ntwo",
		},
		{
			name: "new messageId at line start adds nothing",
			events: []protocol.SessionEventVariant{
				protocol.AgentMessageChunk{MessageID: "m1", Text: "one\n"},
				protocol.AgentMessageChunk{MessageID: "m2", Text: "two"},
			},
			want: "one\ntwo",
		},
		{
			name: "thoughts hidden by default",
			events: []protocol.SessionEventVariant{
				protocol.AgentThoughtChunk{MessageID: "t", Text: "hmm"},
				protocol.AgentMessageChunk{MessageID: "m", Text: "answer"},
			},
			want: "answer",
		},
		{
			name:     "thoughts prefixed per line when shown",
			thoughts: true,
			events: []protocol.SessionEventVariant{
				protocol.AgentThoughtChunk{MessageID: "t", Text: "first\nsec"},
				protocol.AgentThoughtChunk{MessageID: "t", Text: "ond\n"},
				protocol.AgentMessageChunk{MessageID: "m", Text: "answer"},
			},
			want: "… first\n… second\nanswer",
		},
		{
			name:     "thought then message with same id still separate",
			thoughts: true,
			events: []protocol.SessionEventVariant{
				protocol.AgentThoughtChunk{MessageID: "x", Text: "think"},
				protocol.AgentMessageChunk{MessageID: "x", Text: "say"},
			},
			want: "… think\nsay",
		},
		{
			name: "tool call on a fresh line",
			events: []protocol.SessionEventVariant{
				protocol.AgentMessageChunk{MessageID: "m", Text: "Let me look"},
				protocol.ToolCall{ToolCallID: "1", Title: "read main.go", Kind: protocol.ToolKindRead, Status: protocol.ToolCallStatusPending},
			},
			want: "Let me look\n⚙ read main.go\n",
		},
		{
			name: "in-progress and pending updates are ignored",
			events: []protocol.SessionEventVariant{
				protocol.ToolCallUpdate{ToolCallID: "1", Status: ptr(protocol.ToolCallStatusInProgress), Content: []protocol.ToolCallContent{{Value: protocol.TextContent{Text: "x"}}}},
				protocol.ToolCallUpdate{ToolCallID: "1"},
			},
			want: "",
		},
		{
			name: "completed with short text",
			events: []protocol.SessionEventVariant{
				protocol.ToolCallUpdate{ToolCallID: "1", Status: ptr(protocol.ToolCallStatusCompleted), Content: []protocol.ToolCallContent{{Value: protocol.TextContent{Text: "a\nb\n"}}}},
			},
			want: "  ✓ done\n    a\n    b\n",
		},
		{
			name: "failed without content",
			events: []protocol.SessionEventVariant{
				protocol.ToolCallUpdate{ToolCallID: "1", Status: ptr(protocol.ToolCallStatusFailed)},
			},
			want: "  ✗ failed\n",
		},
		{
			name: "long text is truncated to 10 lines",
			events: []protocol.SessionEventVariant{
				protocol.ToolCallUpdate{ToolCallID: "1", Status: ptr(protocol.ToolCallStatusCompleted), Content: []protocol.ToolCallContent{{Value: protocol.TextContent{Text: long}}}},
			},
			want: "  ✓ done\n" + strings.Repeat("    line\n", 10) + "    … (3 more lines)\n",
		},
		{
			name: "exactly 10 lines is not truncated",
			events: []protocol.SessionEventVariant{
				protocol.ToolCallUpdate{ToolCallID: "1", Status: ptr(protocol.ToolCallStatusCompleted), Content: []protocol.ToolCallContent{{Value: protocol.TextContent{Text: strings.Repeat("l\n", 10)}}}},
			},
			want: "  ✓ done\n" + strings.Repeat("    l\n", 10),
		},
		{
			name: "diff counts ignore shared lines",
			events: []protocol.SessionEventVariant{
				protocol.ToolCallUpdate{ToolCallID: "1", Status: ptr(protocol.ToolCallStatusCompleted), Content: []protocol.ToolCallContent{
					{Value: protocol.DiffContent{Path: "a.go", OldText: ptr("x\ny\nz\n"), NewText: "x\nY\nY2\nz\n"}},
					{Value: protocol.DiffContent{Path: "new.go", NewText: "1\n2\n3"}},
				}},
			},
			want: "  ✓ done\n    diff a.go (+2 -1 lines)\n    diff new.go (+3 -0 lines)\n",
		},
		{
			name: "end_turn only closes the line",
			events: []protocol.SessionEventVariant{
				protocol.AgentMessageChunk{MessageID: "m", Text: "done"},
				protocol.TurnEnded{StopReason: protocol.StopReasonEndTurn},
			},
			want: "done\n",
		},
		{
			name:   "end_turn at line start prints nothing",
			events: []protocol.SessionEventVariant{protocol.TurnEnded{StopReason: protocol.StopReasonEndTurn}},
			want:   "",
		},
		{
			name: "cancelled turn",
			events: []protocol.SessionEventVariant{
				protocol.AgentMessageChunk{MessageID: "m", Text: "wor"},
				protocol.TurnEnded{StopReason: protocol.StopReasonCancelled},
			},
			want: "wor\n[turn cancelled]\n",
		},
		{
			name:   "turn error",
			events: []protocol.SessionEventVariant{protocol.TurnEnded{StopReason: protocol.StopReasonError, Error: ptr("rate limited")}},
			want:   "[turn error: rate limited]\n",
		},
		{
			name: "app connected and disconnected",
			events: []protocol.SessionEventVariant{
				protocol.AppConnected{App: "browser", Kind: protocol.ConnectorKindApp, V: 2, Tools: []string{"open", "close"}},
				protocol.AppConnected{App: "lsp", Kind: protocol.ConnectorKindService, V: 1},
				protocol.AppDisconnected{App: "browser"},
			},
			want: "[app connected: browser (app v2): open, close]\n[app connected: lsp (service v1)]\n[app disconnected: browser]\n",
		},
		{
			name: "focus with sorted context",
			events: []protocol.SessionEventVariant{
				protocol.FocusChanged{App: "browser", Context: map[string]string{"url": "https://x.dev", "project": "p"}},
				protocol.FocusChanged{App: "overlay"},
			},
			want: "[focus: browser project=p url=https://x.dev]\n[focus: overlay]\n",
		},
		{
			name: "harness and unknown events print nothing",
			events: []protocol.SessionEventVariant{
				protocol.HarnessMessage{Harness: "pi", Message: []byte(`{}`)},
				protocol.UnknownSessionEvent{Tag: "from_the_future", Raw: []byte(`{"type":"from_the_future"}`)},
			},
			want: "",
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			var out strings.Builder
			r := newRenderer(&out, tc.thoughts)
			for _, ev := range tc.events {
				r.Render(protocol.SessionEvent{Value: ev})
			}
			if got := out.String(); got != tc.want {
				t.Errorf("got:\n%q\nwant:\n%q", got, tc.want)
			}
		})
	}
}
