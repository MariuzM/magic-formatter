use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crossbeam_channel::Sender;
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, FileChangeType, InitializeParams, Location, Position, Range, ReferenceParams,
    SemanticTokensParams, TextEdit, Uri,
};
use magic_core::highlight::{TOKEN_MODIFIERS, TOKEN_TYPES, encode};
use magic_core::index::Index;
use magic_core::references::{Target, occurrences, symbol_at};
use magic_core::text::LineIndex;
use magic_core::{FormatContext, Language, ToolPaths};
use serde_json::{Value, json};
use tree_sitter::{Parser, Tree};

use crate::document::Document;
use crate::workspace::{index_roots, path_to_uri, read_source, uri_to_path};
use crate::{LANGUAGES, language_by_id, language_for_path};

#[derive(Clone, Copy)]
struct Features {
    semantic_tokens: bool,
    references:      bool,
}

struct Server {
    sender:   Sender<Message>,
    docs:     HashMap<String, Document>,
    parser:   Parser,
    features: HashMap<&'static str, Features>,
    tools:    ToolPaths,
    index:    Arc<RwLock<Index>>,
}

fn status(sender: &Sender<Message>, message: &str) {
    let line = message.lines().find(|l| !l.trim().is_empty()).unwrap_or(message);
    let note = Notification::new("rustfmtMagic/status".into(), json!({ "message": line }));
    sender.send(note.into()).ok();
}

fn respond(sender: &Sender<Message>, id: RequestId, result: Value) {
    sender.send(Response::new_ok(id, result).into()).ok();
}

fn spawn_request(sender: Sender<Message>, id: RequestId, work: impl FnOnce(&Sender<Message>) -> Value + Send + 'static) {
    std::thread::spawn(move || {
        let response = match catch_unwind(AssertUnwindSafe(|| work(&sender))) {
            Ok(result) => Response::new_ok(id, result),
            Err(_) => Response::new_err(id, ErrorCode::InternalError as i32, "rustfmt-magic: internal error".into()),
        };
        sender.send(response.into()).ok();
    });
}

fn path_string(p: Option<&Value>) -> Option<PathBuf> {
    p.and_then(Value::as_str).filter(|s| !s.is_empty()).map(PathBuf::from)
}

fn full_range(doc: &Document) -> Range {
    let (line, character) = doc.lines.position(&doc.text, doc.text.len());
    Range { start: Position { line: 0, character: 0 }, end: Position { line, character } }
}

fn to_range(text: &str, lines: &LineIndex, (start, end): (usize, usize)) -> Range {
    let (sl, sc) = lines.position(text, start);
    let (el, ec) = lines.position(text, end);
    Range { start: Position { line: sl, character: sc }, end: Position { line: el, character: ec } }
}

fn parse(lang: &dyn Language, text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&lang.grammar()).ok()?;
    parser.parse(text, None)
}

struct OpenFile {
    path: PathBuf,
    uri:  Uri,
    text: String,
    tree: Option<Tree>,
}

impl Server {
    fn needs_tree(&self, lang: &dyn Language) -> bool {
        self.features.get(lang.id()).is_some_and(|f| f.semantic_tokens || f.references)
    }

    fn selector(&self, pick: impl Fn(&Features) -> bool) -> Vec<Value> {
        LANGUAGES
            .iter()
            .filter(|l| self.features.get(l.id()).is_some_and(&pick))
            .map(|l| json!({ "language": l.id() }))
            .collect()
    }

    fn register_capabilities(&self) {
        let all: Vec<Value> = LANGUAGES.iter().map(|l| json!({ "language": l.id() })).collect();
        let mut registrations =
            vec![json!({ "id": "formatting", "method": "textDocument/formatting", "registerOptions": { "documentSelector": all } })];
        let semantic = self.selector(|f| f.semantic_tokens);
        if !semantic.is_empty() {
            registrations.push(json!({
                "id": "semanticTokens",
                "method": "textDocument/semanticTokens",
                "registerOptions": {
                    "documentSelector": semantic,
                    "legend": { "tokenTypes": TOKEN_TYPES, "tokenModifiers": TOKEN_MODIFIERS },
                    "full": true,
                },
            }));
        }
        let references = self.selector(|f| f.references);
        if !references.is_empty() {
            registrations.push(json!({
                "id": "references",
                "method": "textDocument/references",
                "registerOptions": { "documentSelector": references },
            }));
        }
        let request = Request::new(
            RequestId::from("register".to_string()),
            "client/registerCapability".into(),
            json!({ "registrations": registrations }),
        );
        self.sender.send(request.into()).ok();
    }

    fn handle_request(&mut self, req: Request) {
        let id = req.id.clone();
        match req.method.as_str() {
            "textDocument/semanticTokens/full" => match req.extract::<SemanticTokensParams>("textDocument/semanticTokens/full") {
                Ok((id, params)) => self.semantic_tokens(id, params),
                Err(e) => self.invalid(id, e),
            },
            "textDocument/formatting" => match req.extract::<DocumentFormattingParams>("textDocument/formatting") {
                Ok((id, params)) => self.formatting(id, params),
                Err(e) => self.invalid(id, e),
            },
            "textDocument/references" => match req.extract::<ReferenceParams>("textDocument/references") {
                Ok((id, params)) => self.references(id, params),
                Err(e) => self.invalid(id, e),
            },
            _ => {
                let resp = Response::new_err(id, ErrorCode::MethodNotFound as i32, format!("unhandled: {}", req.method));
                self.sender.send(resp.into()).ok();
            }
        }
    }

    fn invalid(&self, id: RequestId, e: impl std::fmt::Debug) {
        let resp = Response::new_err(id, ErrorCode::InvalidParams as i32, format!("{e:?}"));
        self.sender.send(resp.into()).ok();
    }

    fn semantic_tokens(&mut self, id: RequestId, params: SemanticTokensParams) {
        let key = params.text_document.uri.as_str().to_string();
        let data = self.docs.get(&key).and_then(|doc| {
            let tree  = doc.tree.as_ref()?;
            let spans = doc.lang.highlighter().spans(tree, &doc.text);
            Some(encode(&spans, &doc.text, &doc.lines))
        });
        respond(&self.sender, id, json!({ "data": data.unwrap_or_default() }));
    }

    fn formatting(&mut self, id: RequestId, params: DocumentFormattingParams) {
        let key = params.text_document.uri.as_str().to_string();
        let Some(doc) = self.docs.get(&key) else {
            respond(&self.sender, id, Value::Null);
            return;
        };
        let lang   = doc.lang;
        let text   = doc.text.clone();
        let range  = full_range(doc);
        let dir    = uri_to_path(&params.text_document.uri).and_then(|p| p.parent().map(PathBuf::from));
        let tools  = self.tools.clone();
        let indent = if params.options.insert_spaces { " ".repeat(params.options.tab_size as usize) } else { "\t".into() };
        spawn_request(self.sender.clone(), id, move |sender| {
            let ctx = FormatContext { dir: dir.as_deref(), indent, tools: &tools };
            match lang.format(&text, &ctx) {
                Ok(formatted) => {
                    if let Some(w) = &formatted.warning {
                        status(sender, &format!("Rustfmt Magic ({w})"));
                    }
                    if formatted.text == text {
                        json!([])
                    } else {
                        serde_json::to_value(vec![TextEdit { range, new_text: formatted.text }]).unwrap()
                    }
                }
                Err(e) => {
                    status(sender, &format!("Rustfmt Magic: {}", e.lines().find(|l| !l.trim().is_empty()).unwrap_or(&e)));
                    json!([])
                }
            }
        });
    }

    fn references(&mut self, id: RequestId, params: ReferenceParams) {
        let uri = params.text_document_position.text_document.uri;
        let key = uri.as_str().to_string();
        let Some(doc) = self.docs.get(&key) else {
            respond(&self.sender, id, Value::Null);
            return;
        };
        let lang      = doc.lang;
        let pos       = params.text_document_position.position;
        let offset    = doc.lines.offset(&doc.text, pos.line, pos.character);
        let include   = params.context.include_declaration;
        let origin    = OpenFile { path: uri_to_path(&uri).unwrap_or_default(), uri, text: doc.text.clone(), tree: doc.tree.clone() };
        let open: Vec<OpenFile> = self
            .docs
            .iter()
            .filter(|(k, d)| **k != key && d.lang.id() == lang.id())
            .filter_map(|(k, d)| {
                let uri: Uri = k.parse().ok()?;
                Some(OpenFile { path: uri_to_path(&uri)?, uri, text: d.text.clone(), tree: d.tree.clone() })
            })
            .collect();
        let index = self.index.clone();
        spawn_request(self.sender.clone(), id, move |_| {
            serde_json::to_value(find_references(lang, origin, open, offset, include, &index)).unwrap()
        });
    }

    fn handle_notification(&mut self, note: Notification) {
        match note.method.as_str() {
            "textDocument/didOpen" => {
                let Ok(p) = note.extract::<DidOpenTextDocumentParams>("textDocument/didOpen") else { return };
                let lang = language_by_id(&p.text_document.language_id)
                    .or_else(|| uri_to_path(&p.text_document.uri).and_then(|path| language_for_path(&path)));
                let Some(lang) = lang else { return };
                let mut doc = Document::new(lang, p.text_document.text);
                if self.needs_tree(lang) {
                    doc.reparse(&mut self.parser);
                }
                self.docs.insert(p.text_document.uri.as_str().to_string(), doc);
            }
            "textDocument/didChange" => {
                let Ok(p) = note.extract::<DidChangeTextDocumentParams>("textDocument/didChange") else { return };
                let needs = self.docs.get(p.text_document.uri.as_str()).is_some_and(|d| self.needs_tree(d.lang));
                if let Some(doc) = self.docs.get_mut(p.text_document.uri.as_str()) {
                    doc.apply(p.content_changes);
                    if needs {
                        doc.reparse(&mut self.parser);
                    }
                }
            }
            "textDocument/didClose" => {
                let Ok(p) = note.extract::<DidCloseTextDocumentParams>("textDocument/didClose") else { return };
                self.docs.remove(p.text_document.uri.as_str());
            }
            "workspace/didChangeWatchedFiles" => {
                let Ok(p) = note.extract::<DidChangeWatchedFilesParams>("workspace/didChangeWatchedFiles") else { return };
                let changes: Vec<(PathBuf, bool)> = p
                    .changes
                    .into_iter()
                    .filter_map(|c| Some((uri_to_path(&c.uri)?, c.typ == FileChangeType::DELETED)))
                    .filter(|(path, _)| {
                        language_for_path(path).is_some_and(|l| self.features.get(l.id()).is_some_and(|f| f.references))
                    })
                    .collect();
                if changes.is_empty() {
                    return;
                }
                let index = self.index.clone();
                std::thread::spawn(move || {
                    for (path, deleted) in changes {
                        match (deleted, read_source(&path)) {
                            (false, Some(text)) => index.write().unwrap().update(&path, &text),
                            _ => {
                                index.write().unwrap().remove(&path);
                            }
                        }
                    }
                });
            }
            _ => {}
        }
    }
}

fn find_references(
    lang: &'static dyn Language,
    origin: OpenFile,
    open: Vec<OpenFile>,
    offset: usize,
    include_declaration: bool,
    index: &RwLock<Index>,
) -> Vec<Location> {
    let Some(tree) = origin.tree.clone().or_else(|| parse(lang, &origin.text)) else { return Vec::new() };
    let Some(symbol) = symbol_at(lang, &tree, &origin.text, offset) else { return Vec::new() };

    let locate = |file: &OpenFile, tree: &Tree, out: &mut Vec<Location>| {
        let lines = LineIndex::new(&file.text);
        for hit in occurrences(lang, tree, &file.text, &symbol, include_declaration) {
            out.push(Location { uri: file.uri.clone(), range: to_range(&file.text, &lines, hit) });
        }
    };

    let mut out = Vec::new();
    locate(&origin, &tree, &mut out);
    if matches!(symbol.target, Target::Local(_)) {
        return out;
    }

    let mut seen: Vec<PathBuf> = vec![origin.path.clone()];
    for file in &open {
        if file.text.contains(&symbol.name)
            && let Some(tree) = file.tree.clone().or_else(|| parse(lang, &file.text))
        {
            locate(file, &tree, &mut out);
        }
        seen.push(file.path.clone());
    }

    let candidates = index.read().unwrap().files_with(&symbol.name);
    for path in candidates {
        if seen.contains(&path) || language_for_path(&path).is_none_or(|l| l.id() != lang.id()) {
            continue;
        }
        let Some(text) = read_source(&path) else { continue };
        if !text.contains(&symbol.name) {
            continue;
        }
        let Some(uri) = path_to_uri(&path) else { continue };
        let Some(tree) = parse(lang, &text) else { continue };
        locate(&OpenFile { path, uri, text, tree: None }, &tree, &mut out);
    }
    out
}

pub fn run() {
    let (connection, io_threads) = Connection::stdio();
    let Ok((id, params)) = connection.initialize_start() else { return };
    let init: InitializeParams = serde_json::from_value(params).unwrap_or_default();
    let options                = init.initialization_options.unwrap_or(Value::Null);

    let mut features = HashMap::new();
    for lang in LANGUAGES {
        let cfg = &options["languages"][lang.id()];
        features.insert(lang.id(), Features {
            semantic_tokens: cfg["semanticTokens"].as_bool().unwrap_or(true),
            references:      cfg["references"].as_bool().unwrap_or(true),
        });
    }
    let tools = ToolPaths { rustfmt: path_string(options.get("rustfmtPath")), topcoat: path_string(options.get("topcoatPath")) };

    let capabilities = json!({
        "capabilities": { "textDocumentSync": { "openClose": true, "change": 2 } },
        "serverInfo": { "name": "rustfmt-magic", "version": env!("CARGO_PKG_VERSION") },
    });
    if connection.initialize_finish(id, capabilities).is_err() {
        return;
    }

    let mut server = Server {
        sender: connection.sender.clone(),
        docs: HashMap::new(),
        parser: Parser::new(),
        features,
        tools,
        index: Arc::new(RwLock::new(Index::default())),
    };
    server.register_capabilities();

    let roots: Vec<PathBuf> =
        init.workspace_folders.unwrap_or_default().iter().filter_map(|f| uri_to_path(&f.uri)).collect();
    let extensions: Vec<&'static str> = LANGUAGES
        .iter()
        .filter(|l| server.features.get(l.id()).is_some_and(|f| f.references))
        .flat_map(|l| l.extensions().iter().copied())
        .collect();
    index_roots(roots, extensions, server.index.clone());

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req).unwrap_or(true) {
                    break;
                }
                server.handle_request(req);
            }
            Message::Notification(note) => server.handle_notification(note),
            Message::Response(_) => {}
        }
    }
    drop(server);
    drop(connection);
    io_threads.join().ok();
}
