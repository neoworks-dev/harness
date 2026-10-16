package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"math"
	"path/filepath"
	"strings"

	protocol "neoworks.dev/suite/protocol/go"
	"neoworks.dev/suite/sdk-go/tools"
)

const appName = "photo-editor"

const instructions = "Edits one image at a time, the current image. Start with open (an asset:// handle or a local file path). " +
	"Every edit stores its result as a new asset and returns its handle, so earlier handles stay valid; pass a handle to other tools " +
	"(or copy the file it points to into a project). undo steps back to the previous image."

var imageSchema = map[string]any{
	"properties": map[string]any{
		"handle": map[string]any{"type": "string", "description": "asset:// handle of the resulting image"},
		"name":   map[string]any{"type": "string"},
		"width":  map[string]any{"type": "integer"},
		"height": map[string]any{"type": "integer"},
		"mime":   map[string]any{"type": "string"},
	},
	"required": []string{"handle", "name", "width", "height", "mime"},
}

func intProp(desc string) map[string]any {
	return map[string]any{"type": "integer", "description": desc}
}

func schema(required []string, props map[string]any) map[string]any {
	s := map[string]any{"properties": props}
	if len(required) > 0 {
		s["required"] = required
	}
	return s
}

// intArg reads an optional integer argument (JSON numbers arrive as float64).
func intArg(args map[string]any, key string) (value int, present bool, err error) {
	raw, ok := args[key]
	if !ok || raw == nil {
		return 0, false, nil
	}
	f, isNum := raw.(float64)
	if !isNum || f != math.Trunc(f) || math.Abs(f) > 1e9 {
		return 0, true, fmt.Errorf("%s must be an integer (got %v)", key, raw)
	}
	return int(f), true, nil
}

func requireInts(args map[string]any, keys ...string) ([]int, error) {
	out := make([]int, len(keys))
	for i, k := range keys {
		v, ok, err := intArg(args, k)
		switch {
		case err != nil:
			return nil, err
		case !ok:
			return nil, fmt.Errorf("%s is required", k)
		}
		out[i] = v
	}
	return out, nil
}

func stringArg(args map[string]any, key string) (string, error) {
	raw, ok := args[key]
	if !ok || raw == nil {
		return "", nil
	}
	s, isStr := raw.(string)
	if !isStr {
		return "", fmt.Errorf("%s must be a string (got %v)", key, raw)
	}
	return s, nil
}

// done turns an editor outcome into a tool result: the text for guests that
// only read text, the data for templates and agents.
func done(verb string, res imageResult, err error) (tools.Result, error) {
	if err != nil {
		return tools.Result{}, err
	}
	return tools.Result{
		Text:       fmt.Sprintf("%s: %s, %d×%d, %s (%s)", verb, res.Name, res.Width, res.Height, res.Mime, res.Handle),
		Structured: res,
	}, nil
}

// photoTools are the editor's MCP tools.
func photoTools(e *editor) []tools.Tool {
	edit := func(op string, fn func(args map[string]any) (func(*image.NRGBA) (*image.NRGBA, error), error)) tools.Handler {
		return func(ctx context.Context, args map[string]any) (tools.Result, error) {
			do, err := fn(args)
			if err != nil {
				return tools.Result{}, err
			}
			res, err := e.edit(ctx, op, do)
			return done(op, res, err)
		}
	}
	return []tools.Tool{
		{
			Name:        "open",
			Description: "Open an image as the current image: an asset:// handle (from another app, e.g. browser.download) or a local file path (PNG, JPEG, GIF). Forgets the undo history.",
			InputSchema: schema(nil, map[string]any{
				"asset": map[string]any{"type": "string", "description": "asset:// handle of an image"},
				"path":  map[string]any{"type": "string", "description": "absolute path of a local image file"},
			}),
			OutputSchema: imageSchema,
			Handler: func(ctx context.Context, args map[string]any) (tools.Result, error) {
				asset, err := stringArg(args, "asset")
				if err != nil {
					return tools.Result{}, err
				}
				path, err := stringArg(args, "path")
				if err != nil {
					return tools.Result{}, err
				}
				// A handle passed as path is still a handle; a relative path would resolve
				// against this app's directory, which means nothing to the agent.
				if asset == "" && strings.HasPrefix(path, "asset://") {
					asset, path = path, ""
				}
				if path != "" && !filepath.IsAbs(path) {
					return tools.Result{}, fmt.Errorf("path must be absolute, got %q", path)
				}
				var res imageResult
				switch {
				case (asset == "") == (path == ""):
					return tools.Result{}, errors.New("give exactly one of asset or path")
				case asset != "":
					res, err = e.openAsset(ctx, asset)
				default:
					res, err = e.openPath(ctx, path)
				}
				return done("Opened", res, err)
			},
		},
		{
			Name:         "info",
			Description:  "Describe the current image: its asset handle, name, size and type.",
			InputSchema:  schema(nil, map[string]any{}),
			OutputSchema: imageSchema,
			Handler: func(context.Context, map[string]any) (tools.Result, error) {
				res, err := e.info()
				return done("Current image", res, err)
			},
		},
		{
			Name:        "crop",
			Description: "Cut out a rectangle of the current image: its top-left corner (x, y) in pixels and its width and height. Returns the new image.",
			InputSchema: schema([]string{"x", "y", "width", "height"}, map[string]any{
				"x":      intProp("left edge of the region, in pixels from the left"),
				"y":      intProp("top edge of the region, in pixels from the top"),
				"width":  intProp("width of the region, at least 1"),
				"height": intProp("height of the region, at least 1"),
			}),
			OutputSchema: imageSchema,
			Handler: edit("crop", func(args map[string]any) (func(*image.NRGBA) (*image.NRGBA, error), error) {
				v, err := requireInts(args, "x", "y", "width", "height")
				if err != nil {
					return nil, err
				}
				return func(src *image.NRGBA) (*image.NRGBA, error) { return crop(src, v[0], v[1], v[2], v[3]) }, nil
			}),
		},
		{
			Name:        "resize",
			Description: "Scale the current image to a width, a height or both. With only one of them the aspect ratio is kept. Returns the new image.",
			InputSchema: schema(nil, map[string]any{
				"width":  intProp("new width in pixels"),
				"height": intProp("new height in pixels"),
			}),
			OutputSchema: imageSchema,
			Handler: edit("resize", func(args map[string]any) (func(*image.NRGBA) (*image.NRGBA, error), error) {
				w, _, err := intArg(args, "width")
				if err != nil {
					return nil, err
				}
				h, _, err := intArg(args, "height")
				if err != nil {
					return nil, err
				}
				return func(src *image.NRGBA) (*image.NRGBA, error) {
					tw, th, err := fitSize(src.Bounds().Dx(), src.Bounds().Dy(), w, h)
					if err != nil {
						return nil, err
					}
					return resize(src, tw, th), nil
				}, nil
			}),
		},
		{
			Name:        "rotate",
			Description: "Rotate the current image clockwise by 90, 180 or 270 degrees. Returns the new image.",
			InputSchema: schema([]string{"degrees"}, map[string]any{
				"degrees": map[string]any{"type": "integer", "enum": []int{90, 180, 270}, "description": "clockwise rotation"},
			}),
			OutputSchema: imageSchema,
			Handler: edit("rotate", func(args map[string]any) (func(*image.NRGBA) (*image.NRGBA, error), error) {
				v, err := requireInts(args, "degrees")
				if err != nil {
					return nil, err
				}
				return func(src *image.NRGBA) (*image.NRGBA, error) { return rotate(src, v[0]) }, nil
			}),
		},
		{
			Name:        "flip",
			Description: "Mirror the current image, left to right (horizontal) or top to bottom (vertical). Returns the new image.",
			InputSchema: schema([]string{"direction"}, map[string]any{
				"direction": map[string]any{"type": "string", "enum": []string{"horizontal", "vertical"}},
			}),
			OutputSchema: imageSchema,
			Handler: edit("flip", func(args map[string]any) (func(*image.NRGBA) (*image.NRGBA, error), error) {
				dir, err := stringArg(args, "direction")
				if err != nil {
					return nil, err
				}
				return func(src *image.NRGBA) (*image.NRGBA, error) { return flip(src, dir) }, nil
			}),
		},
		{
			Name:         "grayscale",
			Description:  "Turn the current image into shades of gray. Returns the new image.",
			InputSchema:  schema(nil, map[string]any{}),
			OutputSchema: imageSchema,
			Handler: edit("grayscale", func(map[string]any) (func(*image.NRGBA) (*image.NRGBA, error), error) {
				return func(src *image.NRGBA) (*image.NRGBA, error) { return grayscale(src), nil }, nil
			}),
		},
		{
			Name:        "export",
			Description: "Encode the current image as PNG or JPEG and store it as a new asset; the current image is unchanged. Returns the exported file's handle.",
			InputSchema: schema([]string{"format"}, map[string]any{
				"format":  map[string]any{"type": "string", "enum": []string{"png", "jpeg"}},
				"quality": map[string]any{"type": "integer", "minimum": 1, "maximum": 100, "description": "JPEG quality, default 90"},
			}),
			OutputSchema: imageSchema,
			Handler: func(ctx context.Context, args map[string]any) (tools.Result, error) {
				format, err := stringArg(args, "format")
				if err != nil {
					return tools.Result{}, err
				}
				quality, ok, err := intArg(args, "quality")
				if err != nil {
					return tools.Result{}, err
				}
				if !ok {
					quality = 90
				}
				res, err := e.export(ctx, strings.ToLower(format), quality)
				return done("Exported", res, err)
			},
		},
		{
			Name:         "undo",
			Description:  "Go back to the image before the last edit. Returns that image (its handle is the one it had before).",
			InputSchema:  schema(nil, map[string]any{}),
			OutputSchema: imageSchema,
			Handler: func(context.Context, map[string]any) (tools.Result, error) {
				res, err := e.undo()
				return done("Undone", res, err)
			},
		},
	}
}

// templates are the view templates for each tool's result (the structured
// content is `data`): a card with the image, its details and buttons.
func templates() map[string]json.RawMessage {
	card := func(title string, buttons ...string) json.RawMessage {
		all := append(append([]string{}, buttons...),
			`{"type":"button","label":"Copy handle","action":"copy:{{data.handle}}","key":"c"}`)
		return json.RawMessage(`{
  "type": "card",
  "title": "` + title + `",
  "children": [
    {"type": "image", "src": "{{data.handle}}", "caption": "{{data.name}}", "width": "{{data.width}}", "height": "{{data.height}}"},
    {"type": "kv", "items": {"Size": "{{data.width}}×{{data.height}}", "Type": "{{data.mime}}", "Handle": "{{data.handle}}"}},
    {"type": "row", "children": [` + strings.Join(all, ",") + `]}
  ]
}`)
	}
	undo := `{"type":"button","label":"Undo","action":"photo-editor.undo","key":"u"}`
	gray := `{"type":"button","label":"Grayscale","action":"photo-editor.grayscale","key":"g"}`
	rot := `{"type":"button","label":"Rotate 90°","action":"photo-editor.rotate","args":{"degrees":90},"key":"r"}`
	return map[string]json.RawMessage{
		"open":      card("Opened image", gray, rot),
		"info":      card("Current image", undo, gray, rot),
		"crop":      card("Cropped image", undo, gray, rot),
		"resize":    card("Resized image", undo, gray, rot),
		"rotate":    card("Rotated image", undo, gray),
		"flip":      card("Flipped image", undo, gray, rot),
		"grayscale": card("Grayscale image", undo, rot),
		"export":    card("Exported image", undo),
		"undo":      card("Undone", undo, gray, rot),
	}
}

// presence is the app's registration; context says which image is open.
func presence(srv *tools.Server, launch []string, d *doc) protocol.AppPresence {
	title := "Photo editor"
	p := protocol.AppPresence{
		App: appName, Kind: protocol.ConnectorKindApp, Title: &title, Open: true,
		Mcp: srv.Endpoint(), Templates: templates(),
	}
	if len(launch) > 0 {
		p.Launch = &protocol.LaunchSpec{Command: launch}
	}
	if d != nil {
		p.Context = map[string]string{"image": d.asset.Name, "asset": d.asset.Handle}
	}
	return p
}
