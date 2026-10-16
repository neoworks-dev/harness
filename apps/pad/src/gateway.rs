//! Calls the daemon's MCP gateway (`POST ${url}/mcp`) to run a connector's tool, the way a guest
//! agent would: the core tool `tools_call {name, args}`. The gateway is stateless, so there is
//! no handshake, and `Content-Type`/`Accept` are all the transport wants. Used for the lsp
//! service's `diagnostics`. Blocking: run it on a worker.
//!
//! The daemon's address and bearer token are in `daemon.json` next to the bus socket. The token
//! is read when needed (a restarted daemon has a new one), only sent to loopback and never
//! printed.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::editor::{Diagnostic, Severity};

/// The lsp service may wait several seconds for a language server to start and answer.
const TIMEOUT: Duration = Duration::from_secs(45);
const MAX_RESPONSE: usize = 8 * 1024 * 1024;

/// What a tool call returned.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolOutput {
    pub text: String,
    pub structured: Option<Value>,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct Gateway {
    /// `daemon.json`.
    info: PathBuf,
}

impl Gateway {
    /// The gateway of the daemon whose bus socket is `socket`.
    pub fn beside(socket: &Path) -> Gateway {
        Gateway {
            info: socket.with_file_name("daemon.json"),
        }
    }

    #[cfg(test)]
    pub fn info_path(&self) -> &Path {
        &self.info
    }

    /// Runs `name` (`app.tool`) through the gateway's `tools_call`.
    pub fn call_tool(&self, name: &str, args: Value) -> Result<ToolOutput, String> {
        let (url, token) = read_info(&self.info)?;
        let body = tools_call_request(name, args).to_string();
        let response = post(&url, &token, &body)?;
        parse_response(response.status, &response.body)
    }

    /// The lsp service's `diagnostics` for a file.
    pub fn diagnostics(&self, path: &Path) -> Result<Vec<Diagnostic>, String> {
        let out = self.call_tool("lsp.diagnostics", json!({"path": path}))?;
        parse_diagnostics(&out)
    }
}

/// The JSON-RPC request for running connector tool `name` through the gateway.
pub fn tools_call_request(name: &str, args: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "tools_call", "arguments": {"name": name, "args": args}},
    })
}

/// Reads the reply of `tools/call`: JSON, or an event stream carrying one JSON message.
pub fn parse_response(status: u16, body: &str) -> Result<ToolOutput, String> {
    let message = json_message(body);
    if !(200..300).contains(&status) {
        let detail = message
            .as_ref()
            .and_then(|m| m["error"]["message"].as_str())
            .unwrap_or("");
        return Err(match status {
            401 => "the daemon refused the token (is it a new daemon?)".to_string(),
            _ => format!("the gateway answered HTTP {status} {detail}")
                .trim_end()
                .to_string(),
        });
    }
    let message = message.ok_or("the gateway's answer is not JSON")?;
    if let Some(error) = message.get("error") {
        let why = error["message"].as_str().unwrap_or("an error");
        return Err(format!("the gateway: {why}"));
    }
    let result = message
        .get("result")
        .ok_or("the gateway's answer has no result")?;
    let text = result["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    Ok(ToolOutput {
        text,
        structured: result.get("structuredContent").cloned(),
        is_error: result["isError"].as_bool().unwrap_or(false),
    })
}

fn json_message(body: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str(body.trim()) {
        return Some(value);
    }
    // Server-sent events: the message is in a `data:` line.
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .find_map(|data| serde_json::from_str(data.trim()).ok())
}

/// The diagnostics in `lsp.diagnostics`' result. A tool error (no server for the language, the
/// service isn't running, …) becomes its message.
pub fn parse_diagnostics(out: &ToolOutput) -> Result<Vec<Diagnostic>, String> {
    if out.is_error {
        return Err(first_line(&out.text));
    }
    let Some(list) = out
        .structured
        .as_ref()
        .and_then(|s| s["diagnostics"].as_array())
    else {
        return Err("the lsp service sent no diagnostics".into());
    };
    Ok(list
        .iter()
        .map(|d| Diagnostic {
            line: d["line"].as_u64().unwrap_or(1).max(1) as usize,
            column: d["column"].as_u64().unwrap_or(1).max(1) as usize,
            severity: Severity::parse(d["severity"].as_str().unwrap_or("error")),
            message: d["message"].as_str().unwrap_or("").to_string(),
            source: d["source"].as_str().unwrap_or("").to_string(),
        })
        .collect())
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        "an error without a message".into()
    } else {
        line.chars().take(200).collect()
    }
}

/// The daemon's base URL and token from `daemon.json`.
fn read_info(path: &Path) -> Result<(String, String), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|_| format!("the daemon isn't running ({} is missing)", path.display()))?;
    parse_info(&text)
}

fn parse_info(text: &str) -> Result<(String, String), String> {
    let info: Value = serde_json::from_str(text).map_err(|_| "daemon.json is not JSON")?;
    let url = info["url"].as_str().ok_or("daemon.json has no url")?;
    let token = info["token"].as_str().ok_or("daemon.json has no token")?;
    Ok((url.to_string(), token.to_string()))
}

struct Response {
    status: u16,
    body: String,
}

/// `http://127.0.0.1:port` and nothing else: the token must not leave the machine.
fn loopback_address(url: &str) -> Result<String, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or("the daemon's url isn't http://")?;
    let host_port = rest.split('/').next().unwrap_or("");
    let (host, port) = host_port
        .rsplit_once(':')
        .ok_or("the daemon's url has no port")?;
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        return Err("the daemon's url isn't on this machine".into());
    }
    port.parse::<u16>()
        .map_err(|_| "the daemon's port is bad")?;
    Ok(host_port.to_string())
}

fn post(base_url: &str, token: &str, body: &str) -> Result<Response, String> {
    let address = loopback_address(base_url)?;
    let mut stream = TcpStream::connect(&address)
        .map_err(|err| format!("cannot reach the daemon's gateway: {err}"))?;
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
         Connection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("cannot reach the daemon's gateway: {err}"))?;
    let raw = read_response(&mut stream)?;
    parse_http(&raw)
}

/// Reads until the answer is complete. Servers don't always close the connection when asked
/// to, so waiting for the end of the stream could mean waiting for the timeout.
fn read_response(stream: &mut impl Read) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let n = stream
            .read(&mut buffer)
            .map_err(|err| format!("the gateway didn't answer: {err}"))?;
        if n == 0 {
            return Ok(raw);
        }
        raw.extend_from_slice(&buffer[..n]);
        if raw.len() > MAX_RESPONSE {
            return Err("the gateway's answer is too big".into());
        }
        if is_complete(&raw) {
            return Ok(raw);
        }
    }
}

/// Whether `raw` holds a whole HTTP response (by its Content-Length, or its last chunk).
fn is_complete(raw: &[u8]) -> bool {
    let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let body = &raw[split + 4..];
    if head.contains("transfer-encoding: chunked") {
        return body.ends_with(b"0\r\n\r\n");
    }
    head.lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .is_some_and(|length| body.len() >= length)
}

fn parse_http(raw: &[u8]) -> Result<Response, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("the gateway's answer is cut off")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or("the gateway's answer isn't HTTP")?;
    let chunked = lines.any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    let body = &raw[split + 4..];
    let body = if chunked {
        dechunk(body)?
    } else {
        body.to_vec()
    };
    Ok(Response {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn dechunk(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let eol = data
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("the gateway's answer is cut off")?;
        let size_text = String::from_utf8_lossy(&data[..eol]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "the gateway's chunks are bad")?;
        data = &data[eol + 2..];
        if size == 0 {
            return Ok(out);
        }
        if data.len() < size {
            return Err("the gateway's answer is cut off".into());
        }
        out.extend_from_slice(&data[..size]);
        data = data.get(size + 2..).unwrap_or(&[]);
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn the_request_is_a_tools_call_through_the_core_tool() {
        let request = tools_call_request("lsp.diagnostics", json!({"path": "/w/a.ts"}));
        assert_eq!(
            request,
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "tools_call",
                    "arguments": {"name": "lsp.diagnostics", "args": {"path": "/w/a.ts"}}}
            })
        );
    }

    #[test]
    fn the_gateway_sits_beside_the_socket() {
        let gateway = Gateway::beside(Path::new("/run/user/1000/suite/bus.sock"));
        assert_eq!(
            gateway.info_path(),
            Path::new("/run/user/1000/suite/daemon.json")
        );
    }

    #[test]
    fn info_gives_url_and_token() {
        let (url, token) =
            parse_info(r#"{"pid":1,"url":"http://127.0.0.1:4000","token":"abc","socket":"/s"}"#)
                .expect("info");
        assert_eq!(
            (url.as_str(), token.as_str()),
            ("http://127.0.0.1:4000", "abc")
        );
        assert!(parse_info("{}").is_err());
        assert!(parse_info("nope").is_err());
    }

    #[test]
    fn the_token_only_goes_to_loopback() {
        assert_eq!(
            loopback_address("http://127.0.0.1:4000").as_deref(),
            Ok("127.0.0.1:4000")
        );
        assert_eq!(
            loopback_address("http://localhost:1/x").as_deref(),
            Ok("localhost:1")
        );
        assert!(loopback_address("http://example.com:80").is_err());
        assert!(loopback_address("https://127.0.0.1:4000").is_err());
        assert!(loopback_address("http://127.0.0.1").is_err());
        assert!(loopback_address("http://127.0.0.1.evil.com:80").is_err());
    }

    #[test]
    fn responses_parse_as_json_or_events() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"a:1:1: error: x"}],
            "structuredContent":{"path":"/w/a.ts","errors":1,"warnings":0,"diagnostics":[
              {"line":4,"column":7,"severity":"error","message":"Cannot find name 'x'.","source":"ts"},
              {"line":9,"column":1,"severity":"warning","message":"unused","source":"ts"}]}}}"#;
        let out = parse_response(200, body).expect("json");
        assert!(!out.is_error);
        assert_eq!(out.text, "a:1:1: error: x");
        let list = parse_diagnostics(&out).expect("diagnostics");
        assert_eq!(list.len(), 2);
        assert_eq!(
            list[0],
            Diagnostic {
                line: 4,
                column: 7,
                severity: Severity::Error,
                message: "Cannot find name 'x'.".into(),
                source: "ts".into()
            }
        );
        assert_eq!(list[1].severity, Severity::Warning);

        let events = format!("event: message\ndata: {}\n\n", body.replace('\n', " "));
        assert_eq!(parse_response(200, &events).expect("sse"), out);
    }

    #[test]
    fn failures_say_what_went_wrong() {
        let closed = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text",
            "text":"Language servers is not open. Open it with apps_launch.\nMore."}],"isError":true}}"#;
        let out = parse_response(200, closed).expect("a tool error is still a result");
        assert!(out.is_error);
        assert_eq!(
            parse_diagnostics(&out).unwrap_err(),
            "Language servers is not open. Open it with apps_launch."
        );

        assert!(
            parse_response(401, "unauthorized")
                .unwrap_err()
                .contains("token")
        );
        assert!(parse_response(500, "").unwrap_err().contains("500"));
        assert!(
            parse_response(200, "<html>")
                .unwrap_err()
                .contains("not JSON")
        );
        let rpc = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"nope"}}"#;
        assert!(parse_response(200, rpc).unwrap_err().contains("nope"));
        let no_list = ToolOutput {
            text: "ok".into(),
            structured: Some(json!({})),
            is_error: false,
        };
        assert!(parse_diagnostics(&no_list).is_err());
    }

    #[test]
    fn http_answers_split_into_status_and_body() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        let r = parse_http(plain).expect("plain");
        assert_eq!((r.status, r.body.as_str()), (200, "{}"));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n";
        let r = parse_http(chunked).expect("chunked");
        assert_eq!(r.body, "{\"a\":1}");
        assert!(parse_http(b"garbage").is_err());
        assert!(is_complete(plain));
        assert!(!is_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n{}"
        ));
        assert!(is_complete(chunked));
        assert!(!is_complete(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n"
        ));
        assert!(!is_complete(b"HTTP/1.1 200 OK\r\nContent-Le"));
    }

    /// What the fake gateway saw: request line, headers, body.
    type Seen = (String, Vec<String>, String);

    /// A fake gateway on loopback: answers one request and reports what it received.
    fn serve(reply: &'static str) -> (Gateway, mpsc::Receiver<Seen>, PathBuf) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let dir = std::env::temp_dir().join(format!("suite-pad-gw-{port}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(
            dir.join("daemon.json"),
            json!({"url": format!("http://127.0.0.1:{port}"), "token": "t0ken", "pid": 1})
                .to_string(),
        )
        .expect("daemon.json");
        let (seen, received) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut request_line = String::new();
            reader.read_line(&mut request_line).expect("line");
            let request_line = request_line.trim_end().to_string();
            let mut headers = Vec::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("header");
                let line = line.trim_end().to_string();
                if line.is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
                headers.push(line);
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).expect("body");
            seen.send((
                request_line,
                headers,
                String::from_utf8(body).expect("utf8"),
            ))
            .expect("send");
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            );
            stream.write_all(head.as_bytes()).expect("head");
            stream.write_all(reply.as_bytes()).expect("reply");
            // Like Bun, keep the connection open despite `Connection: close`.
            std::thread::sleep(Duration::from_secs(5));
        });
        (Gateway::beside(&dir.join("bus.sock")), received, dir)
    }

    #[test]
    fn diagnostics_go_over_http_with_the_token() {
        let (gateway, received, dir) = serve(
            r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"x"}],
               "structuredContent":{"diagnostics":[{"line":2,"column":3,"severity":"warning","message":"m","source":"s"}]}}}"#,
        );
        let list = gateway
            .diagnostics(Path::new("/w/a.ts"))
            .expect("diagnostics");
        assert_eq!(list.len(), 1);
        assert_eq!(
            (list[0].line, list[0].column, list[0].severity),
            (2, 3, Severity::Warning)
        );

        let (line, headers, body) = received.recv().expect("request");
        assert_eq!(line, "POST /mcp HTTP/1.1");
        assert!(
            headers.iter().any(|h| h == "Authorization: Bearer t0ken"),
            "{headers:?}"
        );
        assert!(
            headers
                .iter()
                .any(|h| h.starts_with("Accept: application/json"))
        );
        let body: Value = serde_json::from_str(&body).expect("json body");
        assert_eq!(body["params"]["arguments"]["name"], "lsp.diagnostics");
        assert_eq!(body["params"]["arguments"]["args"]["path"], "/w/a.ts");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_missing_daemon_is_reported_not_panicked() {
        let gateway = Gateway::beside(Path::new("/nonexistent/suite/bus.sock"));
        let err = gateway.diagnostics(Path::new("/w/a.ts")).unwrap_err();
        assert!(err.contains("isn't running"), "{err}");
    }
}
