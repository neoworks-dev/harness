package sdk

import (
	"fmt"
	"slices"

	protocol "neoworks.dev/neo/protocol/go"
)

// ViewState is a session's view as drawn: the items from a view.snapshot with
// every view.patch applied since. The zero value is empty and ready to use.
//
// It is not safe for concurrent use. Applying a patch never changes the
// components of items handed out earlier (it copies along the path it
// edits), so a renderer may keep what Items returned.
type ViewState struct {
	// Session and Seq are those of the last snapshot or patch applied.
	Session string
	Seq     int64

	items []protocol.ViewItem
}

// Reset replaces everything with the snapshot.
func (v *ViewState) Reset(s protocol.ViewSnapshot) {
	v.Session, v.Seq = s.Session, s.Seq
	v.items = slices.Clone(s.Items)
}

// Items returns the items in order. The slice is a copy.
func (v *ViewState) Items() []protocol.ViewItem { return slices.Clone(v.items) }

// Item returns the item with this ref.
func (v *ViewState) Item(ref string) (protocol.ViewItem, bool) {
	if i := v.index(ref); i >= 0 {
		return v.items[i], true
	}
	return protocol.ViewItem{}, false
}

func (v *ViewState) index(ref string) int {
	return slices.IndexFunc(v.items, func(it protocol.ViewItem) bool { return it.Ref == ref })
}

// Apply applies the patch's ops in order. If an op cannot be applied (the
// state has drifted from the daemon's) the ops before it stay applied, the
// error names the op and Seq stays put; resubscribe to get a fresh snapshot.
// Ops of kinds this version doesn't know are skipped.
func (v *ViewState) Apply(p protocol.ViewPatch) error {
	for i, op := range p.Ops {
		if err := v.applyOp(op); err != nil {
			return fmt.Errorf("view patch seq %d, op %d: %w", p.Seq, i, err)
		}
	}
	v.Seq = p.Seq
	return nil
}

func (v *ViewState) applyOp(op protocol.ViewOp) error {
	switch o := op.Value.(type) {
	case protocol.SetViewOp:
		if i := v.index(o.Item.Ref); i >= 0 {
			v.items[i] = o.Item
		} else {
			v.items = append(v.items, o.Item)
		}
	case protocol.BlocksViewOp:
		i := v.index(o.Ref)
		if i < 0 {
			return fmt.Errorf("blocks: no item %q", o.Ref)
		}
		c, err := editText(v.items[i].Component, o.Path, func(t protocol.TextComponent) (protocol.TextComponent, error) {
			if o.From < 0 || int(o.From) > len(t.Blocks) {
				return t, fmt.Errorf("from %d outside the %d blocks", o.From, len(t.Blocks))
			}
			t.Blocks = append(slices.Clone(t.Blocks[:o.From]), o.Blocks...)
			return t, nil
		})
		if err != nil {
			return fmt.Errorf("blocks %q: %w", o.Ref, err)
		}
		v.items[i].Component = c
	case protocol.RemoveViewOp:
		// Removing what is already gone is not drift: it is what was wanted.
		if i := v.index(o.Ref); i >= 0 {
			v.items = slices.Delete(v.items, i, i+1)
		}
	}
	return nil
}

// editText applies f to the text component at path (child indexes from c)
// and returns the updated tree. Components along the path are copied, never
// modified in place.
func editText(c protocol.Component, path []int64, f func(protocol.TextComponent) (protocol.TextComponent, error)) (protocol.Component, error) {
	if len(path) == 0 {
		t, ok := c.Value.(protocol.TextComponent)
		if !ok {
			return c, fmt.Errorf("target is %T, not a text component", c.Value)
		}
		t, err := f(t)
		return protocol.Component{Value: t}, err
	}
	children, rebuild, ok := withChildren(c)
	if !ok {
		return c, fmt.Errorf("path goes into %T, which has no children", c.Value)
	}
	i := path[0]
	if i < 0 || int(i) >= len(children) {
		return c, fmt.Errorf("child %d of %d", i, len(children))
	}
	child, err := editText(children[i], path[1:], f)
	if err != nil {
		return c, err
	}
	children = slices.Clone(children)
	children[i] = child
	return rebuild(children), nil
}

// withChildren returns the children of a container component and a function
// that returns a copy of the container with other children.
func withChildren(c protocol.Component) ([]protocol.Component, func([]protocol.Component) protocol.Component, bool) {
	switch v := c.Value.(type) {
	case protocol.CardComponent:
		return v.Children, func(cs []protocol.Component) protocol.Component {
			v.Children = cs
			return protocol.Component{Value: v}
		}, true
	case protocol.RowComponent:
		return v.Children, func(cs []protocol.Component) protocol.Component {
			v.Children = cs
			return protocol.Component{Value: v}
		}, true
	case protocol.StackComponent:
		return v.Children, func(cs []protocol.Component) protocol.Component {
			v.Children = cs
			return protocol.Component{Value: v}
		}, true
	}
	return nil, nil, false
}
