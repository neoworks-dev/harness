package tools

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"
)

func testServer(t *testing.T) *Server {
	t.Helper()
	srv, err := Start(Options{
		Name:         "test-app",
		Instructions: "use it well",
		Tools: []Tool{
			{
				Name:        "add",
				Description: "Adds a and b.",
				Annotations: &Annotations{DestructiveHint: new(bool)},
				InputSchema: map[string]any{"properties": map[string]any{"a": map[string]any{"type": "number"}}},
				OutputSchema: map[string]any{
					"properties": map[string]any{"sum": map[string]any{"type": "number"}},
				},
				Handler: func(_ context.Context, args map[string]any) (Result, error) {
					a, _ := args["a"].(float64)
					b, _ := args["b"].(float64)
					return Result{Text: "sum computed", Structured: map[string]any{"sum": a + b}}, nil
				},
			},
			{Name: "fail", Annotations: &Annotations{ReadOnlyHint: true}, Handler: func(context.Context, map[string]any) (Result, error) {
				return Result{}, errors.New("it broke")
			}},
			{Name: "boom", Handler: func(context.Context, map[string]any) (Result, error) { panic("kaboom") }},
			{Name: "only-structured", Handler: func(context.Context, map[string]any) (Result, error) {
				return Result{Structured: struct {
					N int `json:"n"`
				}{7}}, nil
			}},
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { srv.Close() })
	return srv
}

// post sends body with the server's token (unless headers say otherwise).
func post(t *testing.T, srv *Server, body string, headers map[string]string) (*http.Response, []byte) {
	t.Helper()
	req, err := http.NewRequest(http.MethodPost, srv.URL(), strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json, text/event-stream")
	for k, v := range srv.Headers() {
		req.Header.Set(k, v)
	}
	for k, v := range headers {
		if v == "" {
			req.Header.Del(k)
		} else {
			req.Header.Set(k, v)
		}
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	data, _ := io.ReadAll(resp.Body)
	return resp, data
}

func rpc(t *testing.T, srv *Server, body string) map[string]any {
	t.Helper()
	resp, data := post(t, srv, body, nil)
	if resp.StatusCode != 200 || !strings.HasPrefix(resp.Header.Get("Content-Type"), "application/json") {
		t.Fatalf("status %d, type %q: %s", resp.StatusCode, resp.Header.Get("Content-Type"), data)
	}
	var out map[string]any
	if err := json.Unmarshal(data, &out); err != nil {
		t.Fatalf("%v: %s", err, data)
	}
	return out
}

func TestBoundToLoopbackWithToken(t *testing.T) {
	srv := testServer(t)
	if !strings.HasPrefix(srv.URL(), "http://127.0.0.1:") || !strings.HasSuffix(srv.URL(), "/mcp") {
		t.Fatalf("url %q", srv.URL())
	}
	ep := srv.Endpoint()
	if ep.URL != srv.URL() || !strings.HasPrefix(ep.Headers["authorization"], "Bearer ") || len(ep.Headers["authorization"]) < 40 {
		t.Fatalf("endpoint %+v", ep)
	}
	other := testServer(t)
	if other.Headers()["authorization"] == ep.Headers["authorization"] {
		t.Fatal("two servers share a token")
	}
	if _, err := Start(Options{Name: "x", Addr: "0.0.0.0:0"}); err == nil {
		t.Fatal("a non-loopback address was accepted")
	}
}

func TestAuth(t *testing.T) {
	srv := testServer(t)
	ping := `{"jsonrpc":"2.0","id":1,"method":"ping"}`
	for name, h := range map[string]map[string]string{
		"none":  {"Authorization": ""},
		"wrong": {"Authorization": "Bearer nope"},
		"basic": {"Authorization": "Basic " + strings.TrimPrefix(srv.Headers()["authorization"], "Bearer ")},
	} {
		if resp, _ := post(t, srv, ping, h); resp.StatusCode != http.StatusUnauthorized {
			t.Errorf("%s: status %d", name, resp.StatusCode)
		}
	}
	if resp, _ := post(t, srv, ping, nil); resp.StatusCode != 200 {
		t.Errorf("valid token: status %d", resp.StatusCode)
	}
}

func TestInitializeNegotiatesVersion(t *testing.T) {
	srv := testServer(t)
	for _, tc := range []struct{ asked, want string }{
		{"2025-06-18", "2025-06-18"},
		{"2024-11-05", "2024-11-05"},
		{"2099-01-01", "2025-11-25"},
		{"", "2025-11-25"},
	} {
		body := `{"jsonrpc":"2.0","id":"i","method":"initialize","params":{"protocolVersion":"` + tc.asked + `","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}`
		res := rpc(t, srv, body)["result"].(map[string]any)
		if res["protocolVersion"] != tc.want {
			t.Errorf("asked %q: got %v, want %s", tc.asked, res["protocolVersion"], tc.want)
		}
		if res["instructions"] != "use it well" {
			t.Errorf("instructions %v", res["instructions"])
		}
		if res["serverInfo"].(map[string]any)["name"] != "test-app" {
			t.Errorf("serverInfo %v", res["serverInfo"])
		}
		if _, ok := res["capabilities"].(map[string]any)["tools"]; !ok {
			t.Errorf("capabilities %v", res["capabilities"])
		}
	}
	if got := rpc(t, srv, `{"jsonrpc":"2.0","id":"s","method":"initialize"}`)["id"]; got != "s" {
		t.Errorf("id echoed as %v", got)
	}
}

func TestNotificationsAndPing(t *testing.T) {
	srv := testServer(t)
	for _, body := range []string{
		`{"jsonrpc":"2.0","method":"notifications/initialized"}`,
		`{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}`,
		`{"jsonrpc":"2.0","id":3,"result":{}}`,
	} {
		resp, data := post(t, srv, body, nil)
		if resp.StatusCode != http.StatusAccepted || len(data) != 0 {
			t.Errorf("%s: status %d, body %q", body, resp.StatusCode, data)
		}
	}
	out := rpc(t, srv, `{"jsonrpc":"2.0","id":9,"method":"ping"}`)
	if out["id"] != float64(9) || out["error"] != nil || out["result"] == nil {
		t.Errorf("ping: %v", out)
	}
}

func TestListTools(t *testing.T) {
	srv := testServer(t)
	res := rpc(t, srv, `{"jsonrpc":"2.0","id":1,"method":"tools/list"}`)["result"].(map[string]any)
	list := res["tools"].([]any)
	if len(list) != 4 {
		t.Fatalf("%d tools", len(list))
	}
	add := list[0].(map[string]any)
	if add["name"] != "add" || add["description"] != "Adds a and b." {
		t.Errorf("add: %v", add)
	}
	if in := add["inputSchema"].(map[string]any); in["type"] != "object" || in["properties"] == nil {
		t.Errorf("inputSchema %v", in)
	}
	if out := add["outputSchema"].(map[string]any); out["type"] != "object" {
		t.Errorf("outputSchema %v", out)
	}
	fail := list[1].(map[string]any)
	if fail["inputSchema"].(map[string]any)["type"] != "object" || fail["outputSchema"] != nil {
		t.Errorf("fail: %v", fail)
	}
	// The hints the daemon's approvals read; a tool without them sends none.
	if hints := add["annotations"].(map[string]any); hints["destructiveHint"] != false || len(hints) != 1 {
		t.Errorf("add annotations %v", hints)
	}
	if hints := fail["annotations"].(map[string]any); hints["readOnlyHint"] != true || len(hints) != 1 {
		t.Errorf("fail annotations %v", hints)
	}
	if boom := list[2].(map[string]any); boom["annotations"] != nil {
		t.Errorf("boom: %v", boom)
	}
}

func call(t *testing.T, srv *Server, name, args string) map[string]any {
	t.Helper()
	if args == "" {
		args = "{}"
	}
	out := rpc(t, srv, `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"`+name+`","arguments":`+args+`}}`)
	if out["error"] != nil {
		t.Fatalf("rpc error: %v", out["error"])
	}
	return out["result"].(map[string]any)
}

func TestCallTool(t *testing.T) {
	srv := testServer(t)
	res := call(t, srv, "add", `{"a":2,"b":3.5}`)
	if res["isError"] != nil {
		t.Fatalf("%v", res)
	}
	if sc := res["structuredContent"].(map[string]any); sc["sum"] != 5.5 {
		t.Errorf("structuredContent %v", sc)
	}
	blocks := res["content"].([]any)
	if len(blocks) != 1 || blocks[0].(map[string]any)["text"] != "sum computed" || blocks[0].(map[string]any)["type"] != "text" {
		t.Errorf("content %v", blocks)
	}

	// Structured only: the text is its JSON.
	res = call(t, srv, "only-structured", "")
	if text := res["content"].([]any)[0].(map[string]any)["text"]; text != `{"n":7}` {
		t.Errorf("text %v", text)
	}
	if res["structuredContent"].(map[string]any)["n"] != float64(7) {
		t.Errorf("%v", res)
	}
}

func TestCallWithoutArguments(t *testing.T) {
	srv := testServer(t)
	out := rpc(t, srv, `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"add"}}`)
	if out["error"] != nil || out["result"].(map[string]any)["structuredContent"].(map[string]any)["sum"] != float64(0) {
		t.Errorf("%v", out)
	}
}

func TestToolErrors(t *testing.T) {
	srv := testServer(t)
	for _, name := range []string{"fail", "boom"} {
		res := call(t, srv, name, "")
		if res["isError"] != true || res["structuredContent"] != nil {
			t.Errorf("%s: %v", name, res)
		}
		if text := res["content"].([]any)[0].(map[string]any)["text"].(string); name == "fail" && text != "it broke" {
			t.Errorf("text %q", text)
		}
	}
	// The server survives a panicking handler.
	if res := call(t, srv, "add", `{"a":1}`); res["isError"] != nil {
		t.Errorf("%v", res)
	}
}

func TestProtocolErrors(t *testing.T) {
	srv := testServer(t)
	for name, tc := range map[string]struct {
		body string
		code float64
	}{
		"unknown method":     {`{"jsonrpc":"2.0","id":1,"method":"resources/list"}`, codeMethodNotFound},
		"unknown tool":       {`{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nope"}}`, codeInvalidParams},
		"no name":            {`{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}`, codeInvalidParams},
		"arguments array":    {`{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"add","arguments":[1]}}`, codeInvalidParams},
		"arguments string":   {`{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"add","arguments":"x"}}`, codeInvalidParams},
		"not json":           {`{`, codeParse},
		"wrong jsonrpc":      {`{"jsonrpc":"1.0","id":1,"method":"ping"}`, codeInvalidRequest},
		"batch not accepted": {`[{"jsonrpc":"2.0","id":1,"method":"ping"}]`, codeParse},
	} {
		out := rpc(t, srv, tc.body)
		e, _ := out["error"].(map[string]any)
		if e == nil || e["code"] != tc.code || out["result"] != nil {
			t.Errorf("%s: %v", name, out)
		}
	}
}

func TestOtherMethodsRefused(t *testing.T) {
	srv := testServer(t)
	for _, method := range []string{http.MethodGet, http.MethodDelete} {
		req, _ := http.NewRequest(method, srv.URL(), nil)
		for k, v := range srv.Headers() {
			req.Header.Set(k, v)
		}
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		resp.Body.Close()
		if resp.StatusCode != http.StatusMethodNotAllowed {
			t.Errorf("%s: status %d", method, resp.StatusCode)
		}
	}
	req, _ := http.NewRequest(http.MethodPost, strings.TrimSuffix(srv.URL(), "/mcp")+"/other", bytes.NewReader(nil))
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusNotFound {
		t.Errorf("other path: status %d", resp.StatusCode)
	}
}

func TestStartRejectsBadTools(t *testing.T) {
	h := func(context.Context, map[string]any) (Result, error) { return Result{}, nil }
	for name, opts := range map[string]Options{
		"no name":      {Tools: nil},
		"no handler":   {Name: "x", Tools: []Tool{{Name: "a"}}},
		"no tool name": {Name: "x", Tools: []Tool{{Handler: h}}},
		"duplicate":    {Name: "x", Tools: []Tool{{Name: "a", Handler: h}, {Name: "a", Handler: h}}},
	} {
		if srv, err := Start(opts); err == nil {
			srv.Close()
			t.Errorf("%s: no error", name)
		}
	}
}

func TestClose(t *testing.T) {
	srv := testServer(t)
	if err := srv.Close(); err != nil {
		t.Fatal(err)
	}
	if err := srv.Close(); err != nil {
		t.Fatal(err)
	}
	req, _ := http.NewRequest(http.MethodPost, srv.URL(), strings.NewReader(`{}`))
	if resp, err := http.DefaultClient.Do(req); err == nil {
		resp.Body.Close()
		t.Fatal("still serving after Close")
	}
}
