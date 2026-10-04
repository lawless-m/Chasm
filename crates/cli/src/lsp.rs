//! `chasm lsp`: a Language Server Protocol server over stdio.

use std::collections::HashMap;

use chasm_core::{compile, Compilation, Options, Severity, Source};
use lsp_server::{Connection, ErrorCode, Message, Notification, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as _, PublishDiagnostics,
};
use lsp_types::request::{GotoDefinition, HoverRequest, Request as _};
use lsp_types::{
    Diagnostic as LspDiagnostic, DiagnosticSeverity, HoverProviderCapability, NumberOrString,
    OneOf, Position, PublishDiagnosticsParams, Range, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams, Location,
    MarkupContent, MarkupKind,
};

pub fn run() -> Result<(), String> {
    let (connection, io_threads) = Connection::stdio();
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    };
    let caps = serde_json::to_value(capabilities).map_err(|e| e.to_string())?;
    connection.initialize(caps).map_err(|e| e.to_string())?;
    let mut docs: HashMap<String, String> = HashMap::new();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection
                    .handle_shutdown(&req)
                    .map_err(|e| e.to_string())?
                {
                    break;
                }
                let resp = match req.method.as_str() {
                    HoverRequest::METHOD => {
                        let result = serde_json::from_value::<HoverParams>(req.params)
                            .ok()
                            .and_then(|p| hover(&docs, p));
                        Response::new_ok(req.id, result)
                    }
                    GotoDefinition::METHOD => {
                        let result = serde_json::from_value::<GotoDefinitionParams>(req.params)
                            .ok()
                            .and_then(|p| definition(&docs, p));
                        Response::new_ok(req.id, result)
                    }
                    _ => Response::new_err(
                        req.id,
                        ErrorCode::MethodNotFound as i32,
                        format!("unsupported request `{}`", req.method),
                    ),
                };
                send(&connection, Message::Response(resp))?;
            }
            Message::Notification(n) => {
                if let Some(uri) = update(&mut docs, n) {
                    publish(&connection, &docs, uri)?;
                }
            }
            Message::Response(_) => {}
        }
    }
    drop(connection);
    io_threads.join().map_err(|e| e.to_string())
}

fn send(connection: &Connection, msg: Message) -> Result<(), String> {
    connection.sender.send(msg).map_err(|e| e.to_string())
}

/// Apply a document notification; returns the document to re-publish.
/// Documents are keyed by their URI string.
fn update(docs: &mut HashMap<String, String>, n: Notification) -> Option<Uri> {
    let params = n.params;
    match n.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let p: lsp_types::DidOpenTextDocumentParams = serde_json::from_value(params).ok()?;
            docs.insert(p.text_document.uri.to_string(), p.text_document.text);
            Some(p.text_document.uri)
        }
        DidChangeTextDocument::METHOD => {
            let p: lsp_types::DidChangeTextDocumentParams = serde_json::from_value(params).ok()?;
            let text = p.content_changes.into_iter().last()?.text;
            docs.insert(p.text_document.uri.to_string(), text);
            Some(p.text_document.uri)
        }
        DidSaveTextDocument::METHOD => {
            let p: lsp_types::DidSaveTextDocumentParams = serde_json::from_value(params).ok()?;
            if let Some(text) = p.text {
                docs.insert(p.text_document.uri.to_string(), text);
            }
            Some(p.text_document.uri)
        }
        DidCloseTextDocument::METHOD => {
            let p: lsp_types::DidCloseTextDocumentParams = serde_json::from_value(params).ok()?;
            docs.remove(p.text_document.uri.as_str());
            Some(p.text_document.uri)
        }
        _ => None,
    }
}

/// The file name the compiler reports for a document.
fn doc_name(uri: &Uri) -> String {
    let s = uri.as_str();
    s.strip_prefix("file://").unwrap_or(s).to_string()
}

/// Compile one open document as a whole program, with the prelude.
fn compile_doc(uri: &Uri, text: &str) -> Compilation {
    compile(&[Source::new(doc_name(uri), text)], &Options::default())
}

fn publish(
    connection: &Connection,
    docs: &HashMap<String, String>,
    uri: Uri,
) -> Result<(), String> {
    let diagnostics = match docs.get(uri.as_str()) {
        Some(text) => {
            let name = doc_name(&uri);
            compile_doc(&uri, text)
                .diagnostics
                .iter()
                .filter(|d| d.location.file == name)
                .map(to_lsp)
                .collect()
        }
        None => Vec::new(),
    };
    let params = PublishDiagnosticsParams {
        uri,
        diagnostics,
        version: None,
    };
    let n = Notification::new(PublishDiagnostics::METHOD.to_string(), params);
    send(connection, Message::Notification(n))
}

/// A token's range from its 1-based line and column.
fn token_range(line: u32, column: u32, token: &str) -> Range {
    if line == 0 || column == 0 {
        return Range::default();
    }
    let start = Position::new(line - 1, column - 1);
    let end = Position::new(line - 1, column - 1 + token.chars().count() as u32);
    Range::new(start, end)
}

fn to_lsp(d: &chasm_core::Diagnostic) -> LspDiagnostic {
    let l = &d.location;
    let mut message = d.message.clone();
    if let (Some(e), Some(a)) = (&d.expected, &d.actual) {
        message.push_str(&format!(
            "\nexpected: ( {} )\nactual:   ( {} )",
            e.join(" "),
            a.join(" ")
        ));
    }
    if let Some(deps) = d.dependants.as_ref().filter(|v| !v.is_empty()) {
        message.push_str(&format!("\ndependants: {}", deps.join(", ")));
    }
    if let Some(e) = &d.declared_effect {
        message.push_str(&format!("\ndeclared: {e}"));
    }
    LspDiagnostic {
        range: token_range(l.line, l.column, &l.token),
        severity: Some(match d.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code: Some(NumberOrString::String(d.code.clone())),
        source: Some("chasm".to_string()),
        message,
        ..LspDiagnostic::default()
    }
}

/// The token under a 0-based position, with its range.
fn token_at(name: &str, text: &str, pos: Position) -> Option<(String, Range)> {
    let tokens = chasm_core::lexer::lex(name, text).ok()?;
    tokens.into_iter().find_map(|t| {
        let r = token_range(t.line, t.column, &t.text);
        (r.start.line == pos.line
            && r.start.character <= pos.character
            && pos.character < r.end.character)
            .then_some((t.text, r))
    })
}

fn hover(docs: &HashMap<String, String>, p: HoverParams) -> Option<Hover> {
    let doc = p.text_document_position_params;
    let uri = doc.text_document.uri;
    let text = docs.get(uri.as_str())?;
    let (word, range) = token_at(&doc_name(&uri), text, doc.position)?;
    let comp = compile_doc(&uri, text);
    let value = match comp.word(&word) {
        Some(w) => {
            let mut notes: Vec<String> = Vec::new();
            if w.failed {
                notes.push("body failed to check".into());
            } else if !w.resolved {
                notes.push("declared, no body yet".into());
            }
            if w.library {
                notes.push("prelude".into());
            }
            if w.generated {
                notes.push("generated by a struct or union".into());
            }
            if w.export {
                notes.push("export".into());
            }
            if w.inferred {
                notes.push("inferred".into());
            }
            if w.generic {
                notes.push("generic".into());
                let mut instances: Vec<&str> = comp
                    .words
                    .iter()
                    .filter(|i| i.instance_of.as_deref() == Some(w.name.as_str()))
                    .map(|i| i.name.as_str())
                    .collect();
                instances.sort();
                if !instances.is_empty() {
                    notes.push(format!("instances: {}", instances.join(", ")));
                }
            }
            let mut s = format!("```chasm\n{} {}\n```", w.name, w.effect);
            if !notes.is_empty() {
                s.push_str(&format!("\n\n{}", notes.join(", ")));
            }
            s
        }
        None => format!(
            "```chasm\n{word} {}\n```\n\nprimitive",
            primitive_effect(&word)?
        ),
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: Some(range),
    })
}

/// A primitive's effect as written in `docs/reference.md`.
pub(crate) fn primitive_effect(name: &str) -> Option<String> {
    use chasm_core::{prims, types::names};
    if let Some((inputs, outputs, _)) = prims::numeric(name) {
        return Some(effect(&names(&inputs), &names(&outputs)));
    }
    if let Some((inputs, outputs)) = prims::special(name) {
        return Some(effect(&names(&inputs), &names(&outputs)));
    }
    if let Some((n, perm)) = prims::shuffle(name) {
        let letter = |i: usize| ((b'a' + i as u8) as char).to_string();
        let inputs: Vec<String> = (0..n).map(letter).collect();
        let outputs: Vec<String> = perm.iter().map(|&i| letter(i)).collect();
        return Some(effect(&inputs, &outputs));
    }
    let fixed = match name {
        "array.new" => "( i32 -- array T )",
        "array.len" => "( array T -- i32 )",
        "array.at" => "( array T i32 -- T )",
        "array.at!" => "( array T i32 T -- )",
        "array.slice" => "( array T i32 i32 -- array T )",
        "call" => "( ... [ ... -- ... ] -- ... ): calls a function value",
        "leave" => "exits the innermost loop",
        "eq" => "( a a -- i32 ): equal by contents",
        "hash" => "( a -- i32 ): a hash of the contents",
        "if" => "cond [ then ] [ else ] if",
        "when" => "cond [ body ] when",
        "unless" => "cond [ body ] unless",
        "while" => "[ cond ] [ body ] while",
        "until" => "[ body ] [ cond ] until",
        "times" => "n [ body ] times: the body receives the index",
        "each" => "arr [ T -- ] each",
        "map" => "arr [ T -- U ] map: leaves array U",
        "filter" => "arr [ T -- i32 ] filter: leaves array T",
        "fold" => "arr init [ U T -- U ] fold: leaves U",
        "match" => "value v1: [ ... ] v2: [ ... ] else: [ ... ] match",
        _ => return None,
    };
    Some(fixed.to_string())
}

fn effect(inputs: &[String], outputs: &[String]) -> String {
    let side = |v: &[String]| {
        if v.is_empty() {
            String::new()
        } else {
            format!("{} ", v.join(" "))
        }
    };
    format!("( {}-- {})", side(inputs), side(outputs))
}

/// Where a word of the open document is defined: its name in `:`, `declare`
/// or, for generated words, `struct`. Prelude words and primitives have none.
fn definition(
    docs: &HashMap<String, String>,
    p: GotoDefinitionParams,
) -> Option<GotoDefinitionResponse> {
    let doc = p.text_document_position_params;
    let uri = doc.text_document.uri;
    let text = docs.get(uri.as_str())?;
    let name = doc_name(&uri);
    let (word, _) = token_at(&name, text, doc.position)?;
    let comp = compile_doc(&uri, text);
    let l = &comp.word(&word)?.location;
    if l.file != name {
        return None;
    }
    let range = token_range(l.line, l.column, &l.token);
    Some(GotoDefinitionResponse::Scalar(Location { uri, range }))
}
