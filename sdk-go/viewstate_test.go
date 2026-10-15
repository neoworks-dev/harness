package sdk_test

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"

	protocol "neoworks.dev/suite/protocol/go"
	sdk "neoworks.dev/suite/sdk-go"
)

func para(text string) protocol.Block {
	return protocol.Block{Value: protocol.ParagraphBlock{Spans: []protocol.Span{{Text: text}}}}
}

func textOf(blocks ...protocol.Block) protocol.Component {
	return protocol.Component{Value: protocol.TextComponent{Blocks: blocks}}
}

func stackOf(children ...protocol.Component) protocol.Component {
	return protocol.Component{Value: protocol.StackComponent{Children: children}}
}

func cardOf(children ...protocol.Component) protocol.Component {
	return protocol.Component{Value: protocol.CardComponent{Children: children}}
}

func item(ref string, c protocol.Component) protocol.ViewItem {
	return protocol.ViewItem{Ref: ref, Kind: protocol.ItemKindAgent, Component: c}
}

func set(it protocol.ViewItem) protocol.ViewOp {
	return protocol.ViewOp{Value: protocol.SetViewOp{Item: it}}
}

func blocksOp(ref string, path []int64, from int64, blocks ...protocol.Block) protocol.ViewOp {
	return protocol.ViewOp{Value: protocol.BlocksViewOp{Ref: ref, Path: path, From: from, Blocks: blocks}}
}

func remove(ref string) protocol.ViewOp {
	return protocol.ViewOp{Value: protocol.RemoveViewOp{Ref: ref}}
}

func patch(seq int64, ops ...protocol.ViewOp) protocol.ViewPatch {
	return protocol.ViewPatch{Session: "s1", Seq: seq, Ops: ops}
}

func refs(v *sdk.ViewState) string {
	var out []string
	for _, it := range v.Items() {
		out = append(out, it.Ref)
	}
	return strings.Join(out, ",")
}

// blockTexts lists the paragraph texts of the text component at the item's
// component (or inside a container, by path).
func blockTexts(t *testing.T, c protocol.Component, path ...int) []string {
	t.Helper()
	for _, i := range path {
		switch v := c.Value.(type) {
		case protocol.CardComponent:
			c = v.Children[i]
		case protocol.StackComponent:
			c = v.Children[i]
		case protocol.RowComponent:
			c = v.Children[i]
		default:
			t.Fatalf("no children in %T", c.Value)
		}
	}
	var out []string
	for _, b := range c.Value.(protocol.TextComponent).Blocks {
		out = append(out, b.Value.(protocol.ParagraphBlock).Spans[0].Text)
	}
	return out
}

func TestViewStateSnapshotAndSet(t *testing.T) {
	var v sdk.ViewState
	if len(v.Items()) != 0 {
		t.Fatal("zero value not empty")
	}
	v.Reset(protocol.ViewSnapshot{Session: "s1", Seq: 4, Items: []protocol.ViewItem{item("a", textOf(para("A"))), item("b", textOf(para("B")))}})
	if v.Session != "s1" || v.Seq != 4 || refs(&v) != "a,b" {
		t.Fatalf("after reset: %s %d %s", v.Session, v.Seq, refs(&v))
	}

	// set replaces an existing ref in place and appends a new one.
	err := v.Apply(patch(5, set(item("a", textOf(para("A2")))), set(item("c", textOf(para("C"))))))
	if err != nil {
		t.Fatal(err)
	}
	if refs(&v) != "a,b,c" || v.Seq != 5 {
		t.Errorf("refs %s seq %d", refs(&v), v.Seq)
	}
	a, _ := v.Item("a")
	if got := blockTexts(t, a.Component); !reflect.DeepEqual(got, []string{"A2"}) {
		t.Errorf("a = %v", got)
	}
	if _, ok := v.Item("zzz"); ok {
		t.Error("found a missing item")
	}

	// A snapshot replaces everything.
	v.Reset(protocol.ViewSnapshot{Session: "s1", Seq: 9})
	if refs(&v) != "" || v.Seq != 9 {
		t.Errorf("after second reset: %q %d", refs(&v), v.Seq)
	}
}

func TestViewStateBlocksAtRoot(t *testing.T) {
	var v sdk.ViewState
	v.Reset(protocol.ViewSnapshot{Items: []protocol.ViewItem{item("m", textOf(para("one"), para("two")))}})

	// Replace the tail from block 1 and grow the list.
	if err := v.Apply(patch(1, blocksOp("m", nil, 1, para("two!"), para("three")))); err != nil {
		t.Fatal(err)
	}
	m, _ := v.Item("m")
	if got := blockTexts(t, m.Component); !reflect.DeepEqual(got, []string{"one", "two!", "three"}) {
		t.Errorf("blocks = %v", got)
	}
	// from == len appends; from 0 replaces all; an empty list truncates.
	v.Apply(patch(2, blocksOp("m", []int64{}, 3, para("four"))))
	v.Apply(patch(3, blocksOp("m", nil, 2)))
	m, _ = v.Item("m")
	if got := blockTexts(t, m.Component); !reflect.DeepEqual(got, []string{"one", "two!"}) {
		t.Errorf("after truncate = %v", got)
	}
	v.Apply(patch(4, blocksOp("m", nil, 0, para("fresh"))))
	m, _ = v.Item("m")
	if got := blockTexts(t, m.Component); !reflect.DeepEqual(got, []string{"fresh"}) {
		t.Errorf("after replace all = %v", got)
	}
}

func TestViewStateBlocksAtNestedPath(t *testing.T) {
	tree := cardOf(
		textOf(para("header")),
		stackOf(textOf(para("x")), textOf(para("y1"), para("y2"))),
	)
	var v sdk.ViewState
	v.Reset(protocol.ViewSnapshot{Items: []protocol.ViewItem{item("t", tree)}})
	before, _ := v.Item("t")

	if err := v.Apply(patch(1, blocksOp("t", []int64{1, 1}, 1, para("y2!"), para("y3")))); err != nil {
		t.Fatal(err)
	}
	after, _ := v.Item("t")
	card := after.Component.Value.(protocol.CardComponent)
	if got := blockTexts(t, after.Component, 1, 1); !reflect.DeepEqual(got, []string{"y1", "y2!", "y3"}) {
		t.Errorf("nested blocks = %v", got)
	}
	// Siblings are untouched.
	if got := blockTexts(t, after.Component, 0); !reflect.DeepEqual(got, []string{"header"}) {
		t.Errorf("header = %v", got)
	}
	if got := blockTexts(t, after.Component, 1, 0); !reflect.DeepEqual(got, []string{"x"}) {
		t.Errorf("x = %v", got)
	}
	_ = card

	// What was handed out before the patch did not change.
	if got := blockTexts(t, before.Component, 1, 1); !reflect.DeepEqual(got, []string{"y1", "y2"}) {
		t.Errorf("earlier copy mutated: %v", got)
	}
}

func TestViewStateRemove(t *testing.T) {
	var v sdk.ViewState
	v.Reset(protocol.ViewSnapshot{Items: []protocol.ViewItem{item("a", textOf()), item("b", textOf()), item("c", textOf())}})
	if err := v.Apply(patch(1, remove("b"), remove("gone"))); err != nil { // removing what is gone is fine
		t.Fatal(err)
	}
	if refs(&v) != "a,c" {
		t.Errorf("refs = %s", refs(&v))
	}
}

func TestViewStateErrors(t *testing.T) {
	setup := func() *sdk.ViewState {
		v := &sdk.ViewState{}
		v.Reset(protocol.ViewSnapshot{Seq: 7, Items: []protocol.ViewItem{
			item("m", textOf(para("one"))),
			item("t", stackOf(textOf(para("a")), stackOf())),
		}})
		return v
	}
	tests := []struct {
		name string
		op   protocol.ViewOp
		want string
	}{
		{"unknown ref", blocksOp("nope", nil, 0), `no item "nope"`},
		{"from past the end", blocksOp("m", nil, 2, para("x")), "from 2"},
		{"negative from", blocksOp("m", nil, -1), "from -1"},
		{"path into text", blocksOp("m", []int64{0}, 0), "no children"},
		{"child out of range", blocksOp("t", []int64{5}, 0), "child 5 of 2"},
		{"not a text component", blocksOp("t", []int64{1}, 0), "not a text component"},
	}
	for _, tc := range tests {
		v := setup()
		err := v.Apply(patch(8, set(item("ok", textOf())), tc.op))
		if err == nil || !strings.Contains(err.Error(), tc.want) {
			t.Errorf("%s: err = %v, want %q", tc.name, err, tc.want)
		}
		// Ops before the bad one stay applied; Seq does not advance.
		if _, ok := v.Item("ok"); !ok || v.Seq != 7 {
			t.Errorf("%s: partial state: ok=%v seq=%d", tc.name, ok, v.Seq)
		}
	}
}

func TestViewStateSkipsUnknownOps(t *testing.T) {
	var ops []protocol.ViewOp
	if err := json.Unmarshal([]byte(`[{"op":"teleport","ref":"a"},{"op":"remove","ref":"a"}]`), &ops); err != nil {
		t.Fatal(err)
	}
	var v sdk.ViewState
	v.Reset(protocol.ViewSnapshot{Items: []protocol.ViewItem{item("a", textOf())}})
	if err := v.Apply(patch(1, ops...)); err != nil || refs(&v) != "" {
		t.Errorf("err %v, refs %q", err, refs(&v))
	}
}

func TestViewStateFromWireJSON(t *testing.T) {
	// A streaming sequence exactly as the daemon sends it.
	var snap protocol.ViewSnapshot
	var p protocol.ViewPatch
	if err := json.Unmarshal([]byte(`{"session":"s1","seq":2,"items":[{"ref":"m-1","kind":"agent","component":{"type":"text","blocks":[{"type":"paragraph","spans":[{"text":"Hello"}]}]}}]}`), &snap); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal([]byte(`{"session":"s1","seq":3,"ops":[{"op":"blocks","ref":"m-1","path":[],"from":0,"blocks":[{"type":"paragraph","spans":[{"text":"Hello world"}]},{"type":"rule"}]}]}`), &p); err != nil {
		t.Fatal(err)
	}
	var v sdk.ViewState
	v.Reset(snap)
	if err := v.Apply(p); err != nil {
		t.Fatal(err)
	}
	m, _ := v.Item("m-1")
	blocks := m.Component.Value.(protocol.TextComponent).Blocks
	if len(blocks) != 2 || blocks[0].Value.(protocol.ParagraphBlock).Spans[0].Text != "Hello world" {
		t.Errorf("blocks = %+v", blocks)
	}
	if _, ok := blocks[1].Value.(protocol.RuleBlock); !ok {
		t.Errorf("second block %T", blocks[1].Value)
	}
}
