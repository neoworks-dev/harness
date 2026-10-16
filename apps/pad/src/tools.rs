//! The code editor's tools (`open`, `reveal`, `selection`, `files`) and the view templates of
//! their results.
//!
//! The tool server answers on its own threads, so a call is turned into a [`Call`], sent to the
//! GPUI side through a channel and answered from there ([`Request`]); [`apply`] is what the GPUI
//! side runs on the editor model.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};
use suite_sdk::tools::{Tool, ToolResult};

use crate::buffer::Pos;
use crate::editor::{Editor, Effect};

/// How long a tool call waits for the window to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// A tool call, parsed.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Open {
        path: PathBuf,
        line: Option<usize>,
    },
    Reveal {
        path: PathBuf,
        line: usize,
        column: Option<usize>,
    },
    Selection,
    Files,
}

/// A call on its way to the window, with where to send the answer.
pub struct Request {
    pub call: Call,
    pub reply: mpsc::Sender<ToolResult>,
}

/// What [`apply`] did besides answering.
#[derive(Debug, Default)]
pub struct Outcome {
    pub effects: Vec<Effect>,
    /// The line (0-based) the view should scroll to the middle.
    pub reveal: Option<usize>,
}

fn path_arg(args: &Value) -> Result<PathBuf, String> {
    let path = args["path"]
        .as_str()
        .filter(|p| !p.is_empty())
        .ok_or("path is required.")?;
    let path = Path::new(path);
    if path.is_absolute() {
        return Ok(crate::editor::resolve_path(
            path.to_str().unwrap_or(""),
            Path::new("/"),
        ));
    }
    Err(format!("path must be absolute, got {}.", path.display()))
}

fn number_arg(args: &Value, key: &str) -> Result<Option<usize>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|n| *n >= 1)
            .map(|n| Some(n as usize))
            .ok_or_else(|| format!("{key} must be a 1-based number.")),
    }
}

impl Call {
    pub fn parse(tool: &str, args: &Value) -> Result<Call, String> {
        match tool {
            "open" => Ok(Call::Open {
                path: path_arg(args)?,
                line: number_arg(args, "line")?,
            }),
            "reveal" => Ok(Call::Reveal {
                path: path_arg(args)?,
                line: number_arg(args, "line")?.ok_or("line is required (1-based).")?,
                column: number_arg(args, "column")?,
            }),
            "selection" => Ok(Call::Selection),
            "files" => Ok(Call::Files),
            other => Err(format!("unknown tool {other}.")),
        }
    }
}

fn opened(path: &Path, lines: usize, language: &str, line: usize) -> ToolResult {
    ToolResult::structured(
        format!(
            "Opened {} ({lines} lines, {language}), cursor on line {line}.",
            path.display()
        ),
        json!({"path": path, "lines": lines, "language": language, "line": line}),
    )
}

/// Runs a call on the editor. Everything the model can say about the call is in the result.
pub fn apply(editor: &mut Editor, call: Call) -> (ToolResult, Outcome) {
    match call {
        Call::Open { path, line } => match editor.open(&path, line) {
            Ok((info, effects)) => {
                let outcome = Outcome {
                    effects,
                    reveal: line.map(|l| l.saturating_sub(1)),
                };
                (
                    opened(&info.path, info.lines, info.language, info.line),
                    outcome,
                )
            }
            Err(why) => (ToolResult::error(why), Outcome::default()),
        },
        Call::Reveal { path, line, column } => {
            let (info, effects) = match editor.open(&path, None) {
                Ok(ok) => ok,
                Err(why) => return (ToolResult::error(why), Outcome::default()),
            };
            let Some(doc) = editor.current_mut() else {
                return (ToolResult::error("No file is open."), Outcome::default());
            };
            let at = Pos::new(
                (line - 1).min(doc.buffer.line_count() - 1),
                column.map_or(0, |c| c - 1),
            );
            doc.buffer.set_cursor(at, false);
            let at = doc.buffer.cursor();
            doc.highlight = Some(at.line);
            let text = doc.buffer.line(at.line).to_string();
            let result = ToolResult::structured(
                format!(
                    "Showing {}:{}:{}: {}",
                    info.path.display(),
                    at.line + 1,
                    at.col + 1,
                    text.trim()
                ),
                json!({
                    "path": info.path, "line": at.line + 1, "column": at.col + 1,
                    "text": text, "lines": info.lines, "language": info.language,
                }),
            );
            (
                result,
                Outcome {
                    effects,
                    reveal: Some(at.line),
                },
            )
        }
        Call::Selection => (selection(editor), Outcome::default()),
        Call::Files => (files(editor), Outcome::default()),
    }
}

fn selection(editor: &Editor) -> ToolResult {
    let Some(doc) = editor.current() else {
        return ToolResult::structured(
            "No file is open in the code editor.",
            json!({"open": false, "note": "No file is open in the code editor."}),
        );
    };
    let buffer = &doc.buffer;
    let cursor = buffer.cursor();
    let (start, end) = buffer.selection().unwrap_or((cursor, cursor));
    let selected = buffer.selected_text().unwrap_or_default();
    let line_text = buffer.line(cursor.line).to_string();
    let text = if selected.is_empty() {
        format!(
            "{}:{}:{} (no selection). The line: {}",
            doc.path().display(),
            cursor.line + 1,
            cursor.col + 1,
            line_text.trim()
        )
    } else {
        format!(
            "Selected in {} (lines {}-{}):\n{selected}",
            doc.path().display(),
            start.line + 1,
            end.line + 1
        )
    };
    ToolResult::structured(
        text,
        json!({
            "open": true,
            "path": doc.path(),
            "language": doc.language(),
            "line": cursor.line + 1,
            "column": cursor.col + 1,
            "lineText": line_text,
            "text": selected,
            "startLine": start.line + 1,
            "startColumn": start.col + 1,
            "endLine": end.line + 1,
            "endColumn": end.col + 1,
            "dirty": doc.dirty(),
        }),
    )
}

fn files(editor: &Editor) -> ToolResult {
    let current = editor.current_index();
    let files: Vec<Value> = editor
        .docs()
        .iter()
        .enumerate()
        .map(|(i, doc)| {
            json!({
                "path": doc.path(),
                "current": current == Some(i),
                "dirty": doc.dirty(),
                "changedOnDisk": doc.changed_on_disk,
                "state": doc.state(),
                "lines": doc.buffer.line_count(),
                "language": doc.language(),
            })
        })
        .collect();
    let text = if files.is_empty() {
        "No files are open.".to_string()
    } else {
        editor
            .docs()
            .iter()
            .enumerate()
            .map(|(i, d)| {
                format!(
                    "{}{} ({})",
                    if current == Some(i) { "* " } else { "  " },
                    d.path().display(),
                    d.state()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let note = files.is_empty().then_some("No files are open.");
    ToolResult::structured(
        text,
        json!({"count": files.len(), "files": files, "note": note}),
    )
}

fn ask(sender: &async_channel::Sender<Request>, call: Call) -> ToolResult {
    let (reply, answer) = mpsc::channel();
    if sender.send_blocking(Request { call, reply }).is_err() {
        return ToolResult::error("The code editor is closing.");
    }
    answer
        .recv_timeout(ANSWER_TIMEOUT)
        .unwrap_or_else(|_| ToolResult::error("The code editor didn't answer in time."))
}

/// Starts the tool server with the tools, and the channel their calls arrive on.
pub fn serve() -> std::io::Result<(
    suite_sdk::tools::ToolServer,
    async_channel::Receiver<Request>,
)> {
    let (sender, requests) = async_channel::unbounded();
    let server = suite_sdk::tools::ToolServer::start(
        crate::pad::APP,
        Some(
            "The user's code editor. open/reveal show a file or a line to the user; selection tells what they have selected; files lists the open files. Paths are absolute, lines 1-based. After you edit a file the editor reloads it by itself."
                .into(),
        ),
        tools(sender),
    )?;
    Ok((server, requests))
}

/// The tools, each forwarding to the window through `sender`.
pub fn tools(sender: async_channel::Sender<Request>) -> Vec<Tool> {
    let path = json!({"type": "string", "description": "Absolute path of the file."});
    let line = |what: &str| json!({"type": "integer", "minimum": 1, "description": what});
    let tool = |name: &'static str, description: &str, schema: Value| {
        let sender = sender.clone();
        Tool::new(name, description, schema, move |args| {
            match Call::parse(name, &args) {
                Ok(call) => ask(&sender, call),
                Err(why) => ToolResult::error(why),
            }
        })
    };
    vec![
        tool(
            "open",
            "Open a file in the code editor and put the cursor on a line. Returns its path, line count and language.",
            json!({
                "type": "object",
                "properties": {"path": path, "line": line("1-based line for the cursor (default: where it was, or the top).")},
                "required": ["path"],
            }),
        ),
        tool(
            "reveal",
            "Show the user a place in a file: opens it, scrolls there and highlights the line.",
            json!({
                "type": "object",
                "properties": {
                    "path": path,
                    "line": line("1-based line."),
                    "column": line("1-based column (default 1)."),
                },
                "required": ["path", "line"],
            }),
        ),
        tool(
            "selection",
            "What the user is looking at: the current file, the cursor and the selected text. Use it when the user says \"this\" or \"here\".",
            json!({"type": "object", "properties": {}}),
        ),
        tool(
            "files",
            "The files open in the code editor, which one is current, and which have unsaved changes.",
            json!({"type": "object", "properties": {}}),
        ),
    ]
}

/// View templates for the results (protocol/README.md#templates).
pub fn templates() -> std::collections::BTreeMap<String, Value> {
    let file = json!({"type": "file", "path": "{{data.path}}"});
    [
        (
            "open",
            json!({
                "type": "stack",
                "children": [
                    file.clone(),
                    {"type": "kv", "items": {
                        "Line": "{{data.line}}",
                        "Lines": "{{data.lines}}",
                        "Language": "{{data.language}}",
                    }},
                ],
            }),
        ),
        (
            "reveal",
            json!({
                "type": "stack",
                "children": [
                    {"type": "file", "path": "{{data.path}}", "name": "{{data.path}}:{{data.line}}"},
                    {"type": "code", "code": "{{data.text}}", "language": "{{data.language}}"},
                ],
            }),
        ),
        (
            "selection",
            json!({
                "type": "stack",
                "children": [
                    {"type": "text", "if": "data.note", "markdown": "{{data.note}}"},
                    {"type": "file", "if": "data.path", "path": "{{data.path}}",
                     "name": "{{data.path}}:{{data.startLine}}-{{data.endLine}}"},
                    {"type": "code", "if": "data.text", "code": "{{data.text}}",
                     "language": "{{data.language}}"},
                    {"type": "kv", "if": "data.path", "items": {
                        "Cursor": "{{data.line}}:{{data.column}}",
                    }},
                ],
            }),
        ),
        (
            "files",
            json!({
                "type": "stack",
                "children": [
                    {"type": "text", "if": "data.note", "markdown": "{{data.note}}"},
                    {"type": "table", "if": "data.files",
                     "columns": ["path", "state", "lines"], "rows": "{{data.files}}"},
                ],
            }),
        ),
    ]
    .into_iter()
    .map(|(name, template)| (name.to_string(), template))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("suite-pad-tools-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn calls_are_parsed_and_checked() {
        assert_eq!(
            Call::parse("open", &json!({"path": "/a/b.ts", "line": 3})),
            Ok(Call::Open {
                path: "/a/b.ts".into(),
                line: Some(3)
            })
        );
        assert_eq!(
            Call::parse("open", &json!({"path": "/a/../b.ts"})),
            Ok(Call::Open {
                path: "/b.ts".into(),
                line: None
            })
        );
        assert_eq!(
            Call::parse("reveal", &json!({"path": "/a", "line": 2, "column": 5})),
            Ok(Call::Reveal {
                path: "/a".into(),
                line: 2,
                column: Some(5)
            })
        );
        assert_eq!(Call::parse("selection", &json!({})), Ok(Call::Selection));
        assert_eq!(Call::parse("files", &json!({})), Ok(Call::Files));
        assert!(Call::parse("open", &json!({})).is_err());
        assert!(
            Call::parse("open", &json!({"path": "rel/x"}))
                .unwrap_err()
                .contains("absolute")
        );
        assert!(Call::parse("open", &json!({"path": "/a", "line": 0})).is_err());
        assert!(Call::parse("open", &json!({"path": "/a", "line": "x"})).is_err());
        assert!(
            Call::parse("reveal", &json!({"path": "/a"}))
                .unwrap_err()
                .contains("line")
        );
        assert!(Call::parse("nope", &json!({})).is_err());
    }

    #[test]
    fn open_reveal_selection_and_files_on_the_model() {
        let dir = dir("apply");
        let a = dir.join("a.ts");
        let b = dir.join("b.go");
        std::fs::write(&a, "const x = 1;\nconst y = 2;\nconst z = 3;\n").expect("write");
        std::fs::write(&b, "package main\n").expect("write");
        let mut editor = Editor::default();

        let (result, outcome) = apply(&mut editor, Call::Selection);
        assert!(!result.is_error);
        assert_eq!(result.structured.expect("data")["open"], false);
        assert!(outcome.effects.is_empty());

        let (result, outcome) = apply(
            &mut editor,
            Call::Open {
                path: a.clone(),
                line: Some(2),
            },
        );
        assert!(!result.is_error, "{}", result.text);
        let data = result.structured.expect("data");
        assert_eq!(data["path"], a.display().to_string());
        assert_eq!(data["lines"], 3);
        assert_eq!(data["language"], "typescript");
        assert_eq!(data["line"], 2);
        assert_eq!(outcome.effects, [Effect::Diagnose(a.clone())]);
        assert_eq!(outcome.reveal, Some(1));

        // reveal opens another file, puts the cursor and highlights the line.
        let (result, outcome) = apply(
            &mut editor,
            Call::Reveal {
                path: b.clone(),
                line: 1,
                column: Some(9),
            },
        );
        let data = result.structured.expect("data");
        assert_eq!(
            (data["line"].as_u64(), data["column"].as_u64()),
            (Some(1), Some(9))
        );
        assert_eq!(data["text"], "package main");
        assert_eq!(outcome.reveal, Some(0));
        assert_eq!(editor.current().expect("doc").highlight, Some(0));
        // A line past the end is clamped, not an error.
        let (result, _) = apply(
            &mut editor,
            Call::Reveal {
                path: a.clone(),
                line: 99,
                column: None,
            },
        );
        assert_eq!(
            result.structured.expect("data")["line"],
            3,
            "clamped to the last line"
        );

        // selection reports the selected text, over lines.
        let doc = editor.current_mut().expect("doc");
        doc.buffer.set_cursor(Pos::new(0, 6), false);
        doc.buffer.set_cursor(Pos::new(1, 7), true);
        let (result, _) = apply(&mut editor, Call::Selection);
        let data = result.structured.expect("data");
        assert_eq!(data["text"], "x = 1;\nconst y");
        assert_eq!(
            (data["startLine"].as_u64(), data["endLine"].as_u64()),
            (Some(1), Some(2))
        );
        assert!(result.text.contains("x = 1;"), "{}", result.text);

        // files lists both, marks the current one and the unsaved one.
        editor.current_mut().expect("doc").buffer.insert("// ");
        let (result, _) = apply(&mut editor, Call::Files);
        let data = result.structured.expect("data");
        assert_eq!(data["count"], 2);
        let files = data["files"].as_array().expect("files");
        assert_eq!(files[0]["path"], a.display().to_string());
        assert_eq!(files[0]["current"], true);
        assert_eq!(files[0]["dirty"], true);
        assert_eq!(files[1]["dirty"], false);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn open_failures_are_tool_errors() {
        let mut editor = Editor::default();
        let (result, outcome) = apply(
            &mut editor,
            Call::Open {
                path: "/definitely/not/here.ts".into(),
                line: None,
            },
        );
        assert!(result.is_error);
        assert!(result.text.contains("no such file"), "{}", result.text);
        assert!(outcome.effects.is_empty());
        let (result, _) = apply(
            &mut editor,
            Call::Reveal {
                path: "/definitely/not/here.ts".into(),
                line: 1,
                column: None,
            },
        );
        assert!(result.is_error);
    }

    #[test]
    fn the_server_tools_forward_through_the_channel() {
        let (sender, receiver) = async_channel::unbounded::<Request>();
        let tools = tools(sender);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["open", "reveal", "selection", "files"]);

        // Bad arguments never reach the window.
        let open = tools.iter().find(|t| t.name == "open").expect("open");
        let bad = (open.handler)(json!({"path": "relative"}));
        assert!(bad.is_error);
        assert!(receiver.try_recv().is_err());

        // A good call is answered by whoever reads the channel.
        let answerer = std::thread::spawn(move || {
            let request = receiver.recv_blocking().expect("request");
            assert_eq!(
                request.call,
                Call::Open {
                    path: "/a.ts".into(),
                    line: Some(4)
                }
            );
            request.reply.send(ToolResult::text("done")).expect("reply");
        });
        let ok = (open.handler)(json!({"path": "/a.ts", "line": 4}));
        assert_eq!(ok.text, "done");
        answerer.join().expect("answerer");

        // With nobody listening the call fails instead of hanging.
        let gone = (open.handler)(json!({"path": "/a.ts"}));
        assert!(gone.is_error);
        assert!(gone.text.contains("closing"));
    }

    #[test]
    fn every_tool_has_a_template_and_a_schema() {
        let (sender, _receiver) = async_channel::unbounded::<Request>();
        let templates = templates();
        for tool in tools(sender) {
            assert!(templates.contains_key(&tool.name), "{}", tool.name);
            assert_eq!(tool.input_schema["type"], "object");
        }
    }
}
