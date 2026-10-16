package tools

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"slices"
	"strings"
	"sync"
	"time"

	protocol "neoworks.dev/suite/protocol/go"
)

// Protocol versions this server speaks, newest first. initialize echoes the
// client's version when it is one of these, else answers with the newest.
var supportedVersions = []string{"2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"}

// maxBody bounds a request; tool arguments are small.
const maxBody = 8 << 20

// JSON-RPC error codes.
const (
	codeParse          = -32700
	codeInvalidRequest = -32600
	codeMethodNotFound = -32601
	codeInvalidParams  = -32602
)

// Handler runs a tool. args is the call's arguments object (empty, never
// nil). A returned error becomes an isError text result with the error's
// message; return a Result with IsError for failures that carry data.
type Handler func(ctx context.Context, args map[string]any) (Result, error)

// Tool is one tool of the server.
type Tool struct {
	// Name is the tool's name; the gateway prefixes it with the app's name.
	Name        string
	Description string
	// InputSchema is a JSON Schema for the arguments. A missing "type" is
	// filled in as "object"; nil means no properties.
	InputSchema map[string]any
	// OutputSchema describes Result.Structured; optional.
	OutputSchema map[string]any
	Handler      Handler
}

// Content is one block of a result's content. Only text is produced by
// Result.Text; use Content for others (images: Type "image", Data base64,
// MimeType).
type Content struct {
	Type     string `json:"type"`
	Text     string `json:"text,omitempty"`
	Data     string `json:"data,omitempty"`
	MimeType string `json:"mimeType,omitempty"`
}

// Result is what a handler returns.
type Result struct {
	// Text becomes a text content block. When empty and Structured is set,
	// the JSON of Structured is used, as the MCP spec recommends.
	Text string
	// Content are further blocks, after Text.
	Content []Content
	// Structured is sent as structuredContent; it should marshal to an
	// object and match the tool's OutputSchema. The gateway hands it to view
	// templates as `data`.
	Structured any
	IsError    bool
}

// Options configure Start.
type Options struct {
	// Name and Version identify the server in initialize (Version defaults to 0.1.0).
	Name, Version string
	// Instructions goes into the initialize result; the gateway shows it
	// next to the app's tools.
	Instructions string
	Tools        []Tool
	// Addr is the listen address; empty means 127.0.0.1:0 (a free port).
	// It must be a loopback address.
	Addr string
	// Logf receives diagnostics (never tokens); nil discards them.
	Logf func(format string, args ...any)
}

// Server is a running tool server.
type Server struct {
	opts   Options
	token  string
	url    string
	ln     net.Listener
	http   *http.Server
	tools  []Tool
	byName map[string]*Tool

	closeOnce sync.Once
}

// Start validates the tools, listens and serves in the background.
func Start(opts Options) (*Server, error) {
	if opts.Name == "" {
		return nil, errors.New("tools: Options.Name is required")
	}
	if opts.Version == "" {
		opts.Version = "0.1.0"
	}
	s := &Server{opts: opts, tools: slices.Clone(opts.Tools), byName: map[string]*Tool{}}
	for i := range s.tools {
		t := &s.tools[i]
		if t.Name == "" || t.Handler == nil {
			return nil, fmt.Errorf("tools: tool %d needs a Name and a Handler", i)
		}
		if _, dup := s.byName[t.Name]; dup {
			return nil, fmt.Errorf("tools: duplicate tool %q", t.Name)
		}
		s.byName[t.Name] = t
	}
	var raw [32]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return nil, err
	}
	s.token = hex.EncodeToString(raw[:])

	addr := opts.Addr
	if addr == "" {
		addr = "127.0.0.1:0"
	}
	host, _, err := net.SplitHostPort(addr)
	if err != nil {
		return nil, fmt.Errorf("tools: %w", err)
	}
	if ip := net.ParseIP(host); host != "localhost" && (ip == nil || !ip.IsLoopback()) {
		return nil, fmt.Errorf("tools: %q is not a loopback address", addr)
	}
	s.ln, err = net.Listen("tcp", addr)
	if err != nil {
		return nil, err
	}
	s.url = "http://" + s.ln.Addr().String() + "/mcp"

	mux := http.NewServeMux()
	mux.HandleFunc("/mcp", s.serve)
	s.http = &http.Server{Handler: mux, ReadHeaderTimeout: 10 * time.Second}
	go func() {
		if err := s.http.Serve(s.ln); err != nil && !errors.Is(err, http.ErrServerClosed) {
			s.logf("tool server: %v", err)
		}
	}()
	return s, nil
}

// URL is the MCP endpoint, e.g. http://127.0.0.1:41234/mcp.
func (s *Server) URL() string { return s.url }

// Headers are the headers every request must carry (the bearer token).
func (s *Server) Headers() map[string]string {
	return map[string]string{"authorization": "Bearer " + s.token}
}

// Endpoint is the URL and headers, ready for protocol.AppPresence.Mcp.
func (s *Server) Endpoint() *protocol.McpEndpoint {
	return &protocol.McpEndpoint{URL: s.url, Headers: s.Headers()}
}

// Close stops the server; calls in flight get a second to finish.
func (s *Server) Close() error {
	var err error
	s.closeOnce.Do(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		if err = s.http.Shutdown(ctx); err != nil {
			err = s.http.Close()
		}
	})
	return err
}

func (s *Server) logf(format string, args ...any) {
	if s.opts.Logf != nil {
		s.opts.Logf(format, args...)
	}
}

func (s *Server) authorized(r *http.Request) bool {
	given, ok := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
	return ok && subtle.ConstantTimeCompare([]byte(given), []byte(s.token)) == 1
}

// request is a JSON-RPC message. ID stays raw: it is echoed as it came.
type request struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Method  string          `json:"method,omitempty"`
	Params  json.RawMessage `json:"params,omitempty"`
	// Result and Error mark a response (a client answering a request we
	// never send); it is accepted and dropped.
	Result json.RawMessage `json:"result,omitempty"`
	Error  json.RawMessage `json:"error,omitempty"`
}

type rpcError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

type response struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Result  any             `json:"result,omitempty"`
	Error   *rpcError       `json:"error,omitempty"`
}

func (s *Server) serve(w http.ResponseWriter, r *http.Request) {
	if !s.authorized(r) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if r.Method != http.MethodPost {
		// No standalone SSE stream and no sessions to delete.
		w.Header().Set("Allow", "POST")
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, maxBody))
	if err != nil {
		s.reply(w, response{ID: json.RawMessage("null"), Error: &rpcError{codeParse, "cannot read the request body"}})
		return
	}
	var req request
	if err := json.Unmarshal(body, &req); err != nil {
		s.reply(w, response{ID: json.RawMessage("null"), Error: &rpcError{codeParse, "invalid JSON"}})
		return
	}
	hasID := len(req.ID) > 0 && string(req.ID) != "null"
	switch {
	case req.Method == "" && (req.Result != nil || req.Error != nil):
		w.WriteHeader(http.StatusAccepted)
	case req.JSONRPC != "2.0" || req.Method == "":
		id := req.ID
		if !hasID {
			id = json.RawMessage("null")
		}
		s.reply(w, response{ID: id, Error: &rpcError{codeInvalidRequest, "not a JSON-RPC 2.0 request"}})
	case !hasID:
		// A notification (notifications/initialized, cancelled, ...): nothing to answer.
		w.WriteHeader(http.StatusAccepted)
	default:
		result, rerr := s.dispatch(r.Context(), req)
		s.reply(w, response{ID: req.ID, Result: result, Error: rerr})
	}
}

func (s *Server) reply(w http.ResponseWriter, resp response) {
	resp.JSONRPC = "2.0"
	if resp.Error == nil && resp.Result == nil {
		resp.Result = struct{}{}
	}
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(resp); err != nil {
		s.logf("tool server: writing reply: %v", err)
	}
}

func (s *Server) dispatch(ctx context.Context, req request) (any, *rpcError) {
	switch req.Method {
	case "initialize":
		return s.initialize(req.Params), nil
	case "ping":
		return struct{}{}, nil
	case "tools/list":
		return map[string]any{"tools": s.list()}, nil
	case "tools/call":
		return s.call(ctx, req.Params)
	}
	return nil, &rpcError{codeMethodNotFound, "method not found: " + req.Method}
}

func (s *Server) initialize(params json.RawMessage) any {
	var p struct {
		ProtocolVersion string `json:"protocolVersion"`
	}
	_ = json.Unmarshal(params, &p) // a missing version gets the newest
	version := supportedVersions[0]
	if slices.Contains(supportedVersions, p.ProtocolVersion) {
		version = p.ProtocolVersion
	}
	result := map[string]any{
		"protocolVersion": version,
		"capabilities":    map[string]any{"tools": map[string]any{"listChanged": false}},
		"serverInfo":      map[string]any{"name": s.opts.Name, "version": s.opts.Version},
	}
	if s.opts.Instructions != "" {
		result["instructions"] = s.opts.Instructions
	}
	return result
}

func (s *Server) list() []map[string]any {
	out := make([]map[string]any, 0, len(s.tools))
	for _, t := range s.tools {
		schema := map[string]any{}
		for k, v := range t.InputSchema {
			schema[k] = v
		}
		if _, ok := schema["type"]; !ok {
			schema["type"] = "object"
		}
		entry := map[string]any{"name": t.Name, "description": t.Description, "inputSchema": schema}
		if t.OutputSchema != nil {
			outSchema := map[string]any{}
			for k, v := range t.OutputSchema {
				outSchema[k] = v
			}
			if _, ok := outSchema["type"]; !ok {
				outSchema["type"] = "object"
			}
			entry["outputSchema"] = outSchema
		}
		out = append(out, entry)
	}
	return out
}

func (s *Server) call(ctx context.Context, raw json.RawMessage) (any, *rpcError) {
	var p struct {
		Name      string          `json:"name"`
		Arguments json.RawMessage `json:"arguments"`
	}
	if err := json.Unmarshal(raw, &p); err != nil || p.Name == "" {
		return nil, &rpcError{codeInvalidParams, "tools/call needs a tool name"}
	}
	t := s.byName[p.Name]
	if t == nil {
		return nil, &rpcError{codeInvalidParams, "unknown tool: " + p.Name}
	}
	// Arguments are validated loosely: an object (or absent) is required;
	// the gateway already checked them against the input schema.
	args := map[string]any{}
	if len(p.Arguments) > 0 && string(p.Arguments) != "null" {
		if err := json.Unmarshal(p.Arguments, &args); err != nil || args == nil {
			return nil, &rpcError{codeInvalidParams, "arguments must be an object"}
		}
	}
	res, err := s.run(ctx, t, args)
	if err != nil {
		res = Result{Text: err.Error(), IsError: true}
	}
	return encodeResult(res), nil
}

// run calls the handler, turning a panic into an error result.
func (s *Server) run(ctx context.Context, t *Tool, args map[string]any) (res Result, err error) {
	defer func() {
		if p := recover(); p != nil {
			s.logf("tool %s panicked: %v", t.Name, p)
			res, err = Result{}, fmt.Errorf("%s failed unexpectedly", t.Name)
		}
	}()
	return t.Handler(ctx, args)
}

func encodeResult(res Result) map[string]any {
	content := []Content{}
	text := res.Text
	var structured any
	if res.Structured != nil {
		// Round-trip so structs become the plain objects the client sees.
		if data, err := json.Marshal(res.Structured); err == nil {
			_ = json.Unmarshal(data, &structured)
			if text == "" && len(res.Content) == 0 {
				text = string(data)
			}
		}
	}
	if text != "" {
		content = append(content, Content{Type: "text", Text: text})
	}
	content = append(content, res.Content...)
	out := map[string]any{"content": content}
	if _, isObject := structured.(map[string]any); isObject && !res.IsError {
		out["structuredContent"] = structured
	}
	if res.IsError {
		out["isError"] = true
	}
	return out
}
