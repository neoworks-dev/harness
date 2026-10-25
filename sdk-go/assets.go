package sdk

import (
	"context"

	protocol "neoworks.dev/neo/protocol/go"
)

// PutAsset stores a file or bytes in the daemon's asset store and returns
// its handle and location. Exactly one of put.Path (an absolute path, copied
// in) or put.Data (base64) is set. Identical content gets the same handle,
// and a handle stays valid after this app disconnects. The asset records the
// name this client gave in Options.Client as its app. The stored file is
// read-only, so copy it before editing.
func (c *Client) PutAsset(ctx context.Context, put protocol.AssetPut) (protocol.Asset, error) {
	res, err := request[protocol.AssetInfoMessage](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.AssetPutMessage{ID: id, Payload: put}
	})
	return res.Payload, err
}

// GetAsset looks a handle (asset://img-0123456789ab) up. An unknown handle is
// an *Error with code not_found.
func (c *Client) GetAsset(ctx context.Context, handle string) (protocol.Asset, error) {
	res, err := request[protocol.AssetInfoMessage](ctx, c, func(id string) protocol.EnvelopeVariant {
		return protocol.AssetGetMessage{ID: id, Payload: protocol.AssetRef{Handle: handle}}
	})
	return res.Payload, err
}

// FileChanged tells the daemon that this app changed the file at path (an
// absolute path), typically after saving it. The daemon names this client as
// the source and passes it on to every other client (Options.OnFileChanged);
// it is not echoed back and has no reply.
func (c *Client) FileChanged(ctx context.Context, path string) error {
	id, ws, _, err := c.begin(ctx)
	if err != nil {
		return err
	}
	c.forget(id) // no reply is coming; only a live connection was needed
	if err := c.write(ws, protocol.FileChangedMessage{Payload: protocol.FileChanged{Path: path}}); err != nil {
		return c.lostError(err)
	}
	return nil
}
