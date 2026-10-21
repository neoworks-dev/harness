//! A minimal MCP tool server (Streamable HTTP, stateless, JSON responses) on loopback, so a
//! Rust app can bring its own tools without a large MCP crate. Hand [`ToolServer::endpoint`]
//! to the daemon in the app's presence; the gateway lists the tools and forwards calls here.
//!
//! Each connection gets its own thread and one request (`Connection: close`). Handlers run on
//! those threads: hop to your UI thread through a channel if a tool needs app state.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use suite_protocol::McpEndpoint;

/// MCP protocol versions this server speaks; the newest is offered when the client asks for an
/// unknown one.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_BODY: usize = 16 * 1024 * 1024;

/// What a tool returns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolResult {
    /// One text block for the model.
    pub text: String,
    /// The typed result (`structuredContent`); the daemon fills the app's view template with it.
    pub structured: Option<Value>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        ToolResult {
            text: text.into(),
            ..ToolResult::default()
        }
    }

    pub fn structured(text: impl Into<String>, structured: Value) -> Self {
        ToolResult {
            text: text.into(),
            structured: Some(structured),
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        ToolResult {
            text: text.into(),
            structured: None,
            is_error: true,
        }
    }
}

type Handler = Arc<dyn Fn(Value) -> ToolResult + Send + Sync>;

/// One tool: its listing and the function that runs it (arguments are a JSON object).
#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments (an object schema).
    pub input_schema: Value,
    /// MCP's hints about the tool (`readOnlyHint`, `destructiveHint`), sent in `tools/list`. The
    /// daemon decides from them whether the agent's call asks the user first: a read never asks, an
    /// edit the app can undo asks only in ask mode, and any other tool asks in every mode but auto.
    pub annotations: Option<Value>,
    pub handler: Handler,
}

impl Tool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        handler: impl Fn(Value) -> ToolResult + Send + Sync + 'static,
    ) -> Self {
        Tool {
            name: name.into(),
            description: description.into(),
            input_schema,
            annotations: None,
            handler: Arc::new(handler),
        }
    }

    /// Marks the tool as changing nothing.
    pub fn read_only(mut self) -> Self {
        self.annotations = Some(json!({ "readOnlyHint": true }));
        self
    }

    /// Marks the tool's changes as ones the app can undo.
    pub fn undoable(mut self) -> Self {
        self.annotations = Some(json!({ "destructiveHint": false }));
        self
    }
}

struct Shared {
    name: String,
    instructions: Option<String>,
    token: String,
    tools: RwLock<Vec<Tool>>,
    stopped: AtomicBool,
}

/// A running tool server. Dropping it stops accepting connections.
pub struct ToolServer {
    shared: Arc<Shared>,
    url: String,
}

impl ToolServer {
    /// Binds 127.0.0.1 on a free port and serves until dropped.
    pub fn start(
        name: impl Into<String>,
        instructions: Option<String>,
        tools: Vec<Tool>,
    ) -> std::io::Result<ToolServer> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/mcp", listener.local_addr()?);
        let shared = Arc::new(Shared {
            name: name.into(),
            instructions,
            token: random_token()?,
            tools: RwLock::new(tools),
            stopped: AtomicBool::new(false),
        });
        let accept = Arc::clone(&shared);
        thread::Builder::new()
            .name("suite-tools".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if accept.stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let shared = Arc::clone(&accept);
                    let _ = thread::Builder::new()
                        .name("suite-tool-call".into())
                        .spawn(move || serve(stream, &shared));
                }
            })?;
        Ok(ToolServer { shared, url })
    }

    /// `{url, headers}` for `AppPresence::mcp`; the headers carry the bearer token.
    pub fn endpoint(&self) -> McpEndpoint {
        let mut headers = std::collections::BTreeMap::new();
        headers.insert(
            "authorization".to_string(),
            format!("Bearer {}", self.shared.token),
        );
        McpEndpoint {
            url: self.url.clone(),
            headers: Some(headers),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Replaces the tool list. The gateway sees it when it next lists the tools (on presence).
    pub fn set_tools(&self, tools: Vec<Tool>) {
        if let Ok(mut slot) = self.shared.tools.write() {
            *slot = tools;
        }
    }
}

impl Drop for ToolServer {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Relaxed);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(
            self.url
                .trim_start_matches("http://")
                .trim_end_matches("/mcp"),
        );
    }
}

fn random_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> Option<HttpRequest> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((key, value)) = header.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return None;
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn respond(mut stream: &TcpStream, status: &str, body: Option<&Value>) {
    let text = body.map(Value::to_string).unwrap_or_default();
    let content_type = if body.is_some() {
        "Content-Type: application/json\r\n"
    } else {
        ""
    };
    let head = format!(
        "HTTP/1.1 {status}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n",
        text.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(text.as_bytes());
    let _ = stream.flush();
}

fn same_token(given: Option<&String>, token: &str) -> bool {
    let Some(given) = given.and_then(|h| h.strip_prefix("Bearer ")) else {
        return false;
    };
    // Constant time in the token's length.
    given.len() == token.len()
        && given
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn serve(stream: TcpStream, shared: &Shared) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Some(request) = read_request(&stream) else {
        respond(&stream, "400 Bad Request", None);
        return;
    };
    if request.path.split('?').next() != Some("/mcp") {
        respond(&stream, "404 Not Found", None);
        return;
    }
    if !same_token(request.headers.get("authorization"), &shared.token) {
        respond(&stream, "401 Unauthorized", None);
        return;
    }
    if request.method != "POST" {
        // No server-initiated stream (GET) and no sessions to end (DELETE).
        respond(&stream, "405 Method Not Allowed", None);
        return;
    }
    let Ok(message) = serde_json::from_slice::<Value>(&request.body) else {
        let error = rpc_error(Value::Null, -32700, "parse error");
        respond(&stream, "400 Bad Request", Some(&error));
        return;
    };
    match handle(shared, &message) {
        Some(reply) => respond(&stream, "200 OK", Some(&reply)),
        None => respond(&stream, "202 Accepted", None),
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The reply to one JSON-RPC message; `None` for notifications.
fn handle(shared: &Shared, message: &Value) -> Option<Value> {
    let method = message.get("method").and_then(Value::as_str);
    let id = message.get("id").cloned();
    let (Some(method), Some(id)) = (method, id) else {
        return None; // a notification or a stray response
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked
                .filter(|v| VERSIONS.contains(v))
                .unwrap_or(VERSIONS[0]);
            let mut result = json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": shared.name, "version": env!("CARGO_PKG_VERSION")},
            });
            if let Some(instructions) = &shared.instructions {
                result["instructions"] = json!(instructions);
            }
            result
        }
        "ping" => json!({}),
        "tools/list" => {
            let tools = shared.tools.read().map(|t| t.clone()).unwrap_or_default();
            let listed: Vec<Value> = tools
                .iter()
                .map(|t| {
                    let mut listed = json!({"name": t.name, "description": t.description, "inputSchema": t.input_schema});
                    if let Some(annotations) = &t.annotations {
                        listed["annotations"] = annotations.clone();
                    }
                    listed
                })
                .collect();
            json!({ "tools": listed })
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            if !args.is_object() {
                return Some(rpc_error(id, -32602, "arguments must be an object"));
            }
            let tool = shared
                .tools
                .read()
                .ok()
                .and_then(|tools| tools.iter().find(|t| t.name == name).cloned());
            let Some(tool) = tool else {
                return Some(rpc_error(id, -32602, &format!("unknown tool {name}")));
            };
            let out = (tool.handler)(args);
            let mut result = json!({
                "content": [{"type": "text", "text": out.text}],
                "isError": out.is_error,
            });
            if let Some(structured) = out.structured {
                result["structuredContent"] = structured;
            }
            result
        }
        _ => return Some(rpc_error(id, -32601, &format!("unknown method {method}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(server: &ToolServer, token: Option<&str>, body: &Value) -> (String, Option<Value>) {
        let addr = server
            .url()
            .trim_start_matches("http://")
            .trim_end_matches("/mcp");
        let mut stream = TcpStream::connect(addr).expect("connect");
        let text = body.to_string();
        let auth = token
            .map(|t| format!("Authorization: Bearer {t}\r\n"))
            .unwrap_or_default();
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: x\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{text}",
            text.len()
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read");
        let (head, body) = response.split_once("\r\n\r\n").expect("http response");
        let status = head.lines().next().unwrap_or("").to_string();
        (status, serde_json::from_str(body).ok())
    }

    fn server() -> ToolServer {
        let echo = Tool::new(
            "echo",
            "Echo the text",
            json!({"type": "object", "properties": {"text": {"type": "string"}}}),
            |args| {
                let text = args["text"].as_str().unwrap_or("").to_string();
                if text.is_empty() {
                    return ToolResult::error("text is empty");
                }
                ToolResult::structured(text.clone(), json!({ "text": text }))
            },
        );
        ToolServer::start("test", Some("Echoes.".into()), vec![echo]).expect("start")
    }

    fn token(server: &ToolServer) -> String {
        let headers = server.endpoint().headers.unwrap_or_default();
        headers["authorization"]
            .trim_start_matches("Bearer ")
            .to_string()
    }

    #[test]
    fn initialize_list_and_call() {
        let server = server();
        let token = token(&server);
        let t = Some(token.as_str());

        let (status, reply) = post(
            &server,
            t,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}),
        );
        assert!(status.contains("200"), "{status}");
        let reply = reply.expect("json");
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(reply["result"]["instructions"], "Echoes.");

        let (status, _) = post(
            &server,
            t,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        );
        assert!(status.contains("202"), "{status}");

        let (_, reply) = post(
            &server,
            t,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        );
        assert_eq!(reply.expect("json")["result"]["tools"][0]["name"], "echo");

        let call = |args: Value| {
            post(
                &server,
                t,
                &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "echo", "arguments": args}}),
            )
            .1
            .expect("json")
        };
        let ok = call(json!({"text": "hi"}));
        assert_eq!(ok["result"]["content"][0]["text"], "hi");
        assert_eq!(ok["result"]["structuredContent"]["text"], "hi");
        assert_eq!(ok["result"]["isError"], false);
        assert_eq!(call(json!({}))["result"]["isError"], true);
    }

    #[test]
    fn annotations_reach_the_list() {
        let tool = |name: &str| {
            Tool::new(name, "", json!({"type": "object"}), |_| {
                ToolResult::text("ok")
            })
        };
        let tools = vec![
            tool("read").read_only(),
            tool("undoable").undoable(),
            tool("plain"),
        ];
        let server = ToolServer::start("test", None, tools).expect("start");
        let token = token(&server);
        let (_, reply) = post(
            &server,
            Some(token.as_str()),
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );
        let listed = &reply.expect("json")["result"]["tools"];
        assert_eq!(listed[0]["annotations"], json!({"readOnlyHint": true}));
        assert_eq!(listed[1]["annotations"], json!({"destructiveHint": false}));
        assert!(listed[2].get("annotations").is_none());
    }

    #[test]
    fn rejects_missing_token_and_unknown_things() {
        let server = server();
        let token = token(&server);
        let (status, _) = post(
            &server,
            None,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
        );
        assert!(status.contains("401"), "{status}");
        let (status, _) = post(
            &server,
            Some("wrong"),
            &json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
        );
        assert!(status.contains("401"), "{status}");
        let (_, reply) = post(
            &server,
            Some(&token),
            &json!({"jsonrpc": "2.0", "id": 1, "method": "nope"}),
        );
        assert_eq!(reply.expect("json")["error"]["code"], -32601);
        let (_, reply) = post(
            &server,
            Some(&token),
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "missing"}}),
        );
        assert_eq!(reply.expect("json")["error"]["code"], -32602);
    }
}
