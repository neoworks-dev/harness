package tools_test

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	protocol "neoworks.dev/neo/protocol/go"
	sdk "neoworks.dev/neo/sdk-go"
	"neoworks.dev/neo/sdk-go/sdktest"
	"neoworks.dev/neo/sdk-go/tools"
)

func start(t *testing.T) (*sdktest.RealDaemon, *sdk.Client, *tools.Server) {
	t.Helper()
	d := sdktest.StartReal(t)
	srv, err := tools.Start(tools.Options{
		Name:         "adder",
		Instructions: "Adds numbers; call add.",
		Tools: []tools.Tool{
			{
				Name:        "add",
				Description: "Adds two numbers.",
				InputSchema: map[string]any{
					"properties": map[string]any{"a": map[string]any{"type": "number"}, "b": map[string]any{"type": "number"}},
					"required":   []string{"a", "b"},
				},
				OutputSchema: map[string]any{"properties": map[string]any{"sum": map[string]any{"type": "number"}}},
				Handler: func(_ context.Context, args map[string]any) (tools.Result, error) {
					a, _ := args["a"].(float64)
					b, _ := args["b"].(float64)
					return tools.Result{Text: "added", Structured: map[string]any{"sum": a + b}}, nil
				},
			},
			{Name: "fail", Description: "Always fails.", Handler: func(context.Context, map[string]any) (tools.Result, error) {
				return tools.Result{}, errors.New("nope, said the adder")
			}},
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { srv.Close() })

	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	t.Cleanup(cancel)
	c, err := sdk.Dial(ctx, sdk.Options{Socket: d.Socket, Client: "adder"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	title := "Adder"
	if err := c.SetPresence(ctx, protocol.AppPresence{
		App: "adder", Kind: protocol.ConnectorKindApp, Title: &title, Open: true, Mcp: srv.Endpoint(),
	}); err != nil {
		t.Fatal(err)
	}
	return d, c, srv
}

// The gateway lists the tools over MCP (initialize, tools/list) with the
// bearer token from the presence, and calls them.
func TestGatewayListsAndCallsTools(t *testing.T) {
	d, c, _ := start(t)
	ctx := context.Background()

	sdktest.Eventually(t, "the gateway lists the tools", func() bool {
		list, err := c.ListConnectors(ctx)
		if err != nil {
			return false
		}
		for _, info := range list {
			if info.App == "adder" && info.Connected && len(info.Tools) == 2 && info.Error == nil {
				return true
			}
		}
		return false
	})

	found, err := d.CoreTool(ctx, "tools_search", map[string]any{"query": "add", "app": "adder"})
	if err != nil || found.IsError {
		t.Fatalf("tools_search: %v %+v", err, found)
	}
	for _, want := range []string{"adder.add", "Adds two numbers.", "Adds numbers; call add.", "Output schema"} {
		if !strings.Contains(found.Text(), want) {
			t.Errorf("tools_search output lacks %q:\n%s", want, found.Text())
		}
	}

	res, err := d.CallTool(ctx, "adder.add", map[string]any{"a": 2, "b": 40.5})
	if err != nil || res.IsError {
		t.Fatalf("add: %v %+v", err, res)
	}
	if res.StructuredContent["sum"] != 42.5 || res.Text() != "added" {
		t.Errorf("add result: %+v", res)
	}

	// A handler error is a normal tool error.
	res, err = d.CallTool(ctx, "adder.fail", nil)
	if err != nil || !res.IsError || res.Text() != "nope, said the adder" {
		t.Errorf("fail: %v %+v", err, res)
	}

	// The gateway validates arguments against the input schema before forwarding.
	res, err = d.CallTool(ctx, "adder.add", map[string]any{"a": "x"})
	if err != nil || !res.IsError || !strings.Contains(res.Text(), "Invalid arguments") {
		t.Errorf("invalid call: %v %+v", err, res)
	}
}

// Without the token from the presence the gateway could not list anything:
// a presence pointing at the server with no headers shows an error.
func TestServerRefusesWithoutToken(t *testing.T) {
	d, c, srv := start(t)
	ctx := context.Background()
	bare := protocol.AppPresence{App: "adder", Kind: protocol.ConnectorKindApp, Open: true, Mcp: &protocol.McpEndpoint{URL: srv.URL()}}
	if err := c.SetPresence(ctx, bare); err != nil {
		t.Fatal(err)
	}
	sdktest.Eventually(t, "the gateway reports the refused connection", func() bool {
		list, _ := c.ListConnectors(ctx)
		for _, info := range list {
			if info.App == "adder" && info.Error != nil {
				return true
			}
		}
		return false
	})
	if log := d.Log(); strings.Contains(log, strings.TrimPrefix(srv.Headers()["authorization"], "Bearer ")) {
		t.Error("the daemon log contains the tool server's token")
	}
}
