package sdktest

import (
	"slices"
	"strconv"
	"strings"

	protocol "neoworks.dev/neo/protocol/go"
	sdk "neoworks.dev/neo/sdk-go"
)

// The fake daemon resolves session events into view items by a small fixed
// rule set, enough for clients to show a conversation: user messages, streamed
// agent and thought text, tool calls with their text output, and notices for
// turns that did not end normally. Tests that need richer items (cards,
// buttons) add them with PatchView.

// viewModel is one session's view and the state the rules need.
type viewModel struct {
	state   sdk.ViewState
	streams map[string][]protocol.Block // raw text is re-split on every chunk
	raw     map[string]string
	calls   map[string]*call
}

type call struct {
	title   string
	status  protocol.ToolCallStatus
	content []string
}

func newViewModel(session string) *viewModel {
	m := &viewModel{streams: map[string][]protocol.Block{}, raw: map[string]string{}, calls: map[string]*call{}}
	m.state.Reset(protocol.ViewSnapshot{Session: session})
	return m
}

func paragraphs(text string) []protocol.Block {
	blocks := []protocol.Block{}
	for _, p := range strings.Split(text, "\n\n") {
		if p != "" {
			blocks = append(blocks, protocol.Block{Value: protocol.ParagraphBlock{Spans: []protocol.Span{{Text: p}}}})
		}
	}
	return blocks
}

func textComponent(blocks []protocol.Block, tone *protocol.Tone) protocol.Component {
	return protocol.Component{Value: protocol.TextComponent{Tone: tone, Blocks: blocks}}
}

func set(it protocol.ViewItem) protocol.ViewOp {
	return protocol.ViewOp{Value: protocol.SetViewOp{Item: it}}
}

var toolStates = map[protocol.ToolCallStatus]protocol.ItemState{
	protocol.ToolCallStatusPending:    protocol.ItemStatePending,
	protocol.ToolCallStatusInProgress: protocol.ItemStateRunning,
	protocol.ToolCallStatusCompleted:  protocol.ItemStateDone,
	protocol.ToolCallStatusFailed:     protocol.ItemStateFailed,
}

func (m *viewModel) toolItem(id string, c *call) protocol.ViewItem {
	state := toolStates[c.status]
	children := []protocol.Component{}
	for _, text := range c.content {
		children = append(children, protocol.Component{Value: protocol.CodeComponent{Code: text}})
	}
	return protocol.ViewItem{
		Ref: "c-" + id, Kind: protocol.ItemKindTool, Label: &c.title, State: &state,
		Component: protocol.Component{Value: protocol.StackComponent{Children: children}},
	}
}

// stream handles a chunk of streamed text: the first creates the item, later
// ones replace its blocks from the first one that changed.
func (m *viewModel) stream(ref string, kind protocol.ItemKind, chunk string, tone *protocol.Tone) protocol.ViewOp {
	prev, existing := m.streams[ref]
	m.raw[ref] += chunk
	blocks := paragraphs(m.raw[ref])
	m.streams[ref] = blocks
	if !existing {
		return set(protocol.ViewItem{Ref: ref, Kind: kind, Component: textComponent(blocks, tone)})
	}
	from := 0
	for from < len(prev) && from < len(blocks) && sameBlock(prev[from], blocks[from]) {
		from++
	}
	return protocol.ViewOp{Value: protocol.BlocksViewOp{Ref: ref, Path: []int64{}, From: int64(from), Blocks: blocks[from:]}}
}

func sameBlock(a, b protocol.Block) bool {
	pa, okA := a.Value.(protocol.ParagraphBlock)
	pb, okB := b.Value.(protocol.ParagraphBlock)
	return okA && okB && slices.Equal(pa.Spans, pb.Spans)
}

// opsFor applies an event to the model and returns the resulting ops.
func (m *viewModel) opsFor(rec protocol.SessionEventRecord) []protocol.ViewOp {
	switch e := rec.Event.Value.(type) {
	case protocol.UserMessage:
		var comp protocol.Component = textComponent(paragraphs(e.Text), nil)
		if len(e.Images) > 0 {
			children := []protocol.Component{comp}
			for _, src := range e.Images {
				children = append(children, protocol.Component{Value: protocol.ImageComponent{Src: src}})
			}
			comp = protocol.Component{Value: protocol.StackComponent{Children: children}}
		}
		app := e.App
		return []protocol.ViewOp{set(protocol.ViewItem{
			Ref: "u" + strconv.FormatInt(rec.Seq, 10), Kind: protocol.ItemKindUser, Label: &app, Component: comp,
		})}
	case protocol.AgentMessageChunk:
		return []protocol.ViewOp{m.stream("m-"+e.MessageID, protocol.ItemKindAgent, e.Text, nil)}
	case protocol.AgentThoughtChunk:
		muted := protocol.ToneMuted
		return []protocol.ViewOp{m.stream("t-"+e.MessageID, protocol.ItemKindThought, e.Text, &muted)}
	case protocol.ToolCall:
		c := &call{title: e.Title, status: e.Status}
		m.calls[e.ToolCallID] = c
		return []protocol.ViewOp{set(m.toolItem(e.ToolCallID, c))}
	case protocol.ToolCallUpdate:
		c := m.calls[e.ToolCallID]
		if c == nil {
			return nil
		}
		if e.Status != nil {
			c.status = *e.Status
		}
		for _, content := range e.Content {
			if t, ok := content.Value.(protocol.TextContent); ok {
				c.content = append(c.content, strings.TrimRight(t.Text, "\n"))
			}
		}
		return []protocol.ViewOp{set(m.toolItem(e.ToolCallID, c))}
	case protocol.TurnEnded:
		if e.StopReason == protocol.StopReasonEndTurn {
			return nil
		}
		msg, tone := "stopped: "+string(e.StopReason), protocol.ToneDanger
		if e.Error != nil {
			msg = *e.Error
		}
		return []protocol.ViewOp{set(protocol.ViewItem{
			Ref: "e" + strconv.FormatInt(rec.Seq, 10), Kind: protocol.ItemKindNotice, Component: textComponent(paragraphs(msg), &tone),
		})}
	}
	return nil
}
