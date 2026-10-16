// Package tools hosts an app's tools as an MCP server, so the daemon's
// gateway can list and call them.
//
// It is a small hand-written Streamable HTTP server (net/http, no
// dependencies): stateless, plain application/json responses, bound to
// 127.0.0.1 on a free port and guarded by a random bearer token. Hand
// Server.Endpoint to the presence (protocol.AppPresence.Mcp); the gateway
// sends the headers with every request. It is the Go counterpart of sdk-ts's
// createToolServer.
//
//	srv, err := tools.Start(tools.Options{
//	    Name: "photo-editor",
//	    Tools: []tools.Tool{{
//	        Name:        "info",
//	        Description: "Size of the current image.",
//	        Handler: func(ctx context.Context, args map[string]any) (tools.Result, error) {
//	            return tools.Result{Structured: map[string]any{"width": 640}}, nil
//	        },
//	    }},
//	})
//	defer srv.Close()
//	c.SetPresence(ctx, protocol.AppPresence{App: "photo-editor", Open: true, Mcp: srv.Endpoint(), ...})
//
// Stateless means there are no MCP sessions and no server-to-client stream:
// the tool list is fixed once the server started (GET answers 405, so a
// client skips its standalone stream), and calls are answered one request at
// a time with a JSON body.
package tools
