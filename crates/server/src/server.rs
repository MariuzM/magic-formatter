use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crossbeam_channel::Sender;
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, FileChangeType, GotoDefinitionParams, InitializeParams, Location, Position, Range, ReferenceParams,
    SemanticTokensParams, TextEdit, Uri,
};
use magic_core::definitions;
use magic_core::highlight::{TOKEN_MODIFIERS, TOKEN_TYPES, encode, mark_enums, mark_functions, refine_locals};
use magic_core::index::{Index, enum_declarations, type_declarations};
use magic_core::references::{Target, identifier_at, occurrences, symbol_at};
use magic_core::text::LineIndex;
use magic_core::{Analysis, FormatContext, Language, ToolPaths};
use serde_json::{Value, json};
use tree_sitter::{Parser, Tree};

use crate::document::Document;
use crate::sourcekit::SourceKit;
use crate::workspace::{index_roots, outline, path_to_uri, read_source, uri_to_path};

mod features;
use crate::{LANGUAGES, language_by_id, language_for_path};

#[derive(Clone, Copy)]
struct Features {
    semantic_tokens: bool,
    references:      bool,
    language_server: bool,
}

struct Server {
    sender:   Sender<Message>,
    docs:     HashMap<String, Document>,
    parser:   Parser,
    features: HashMap<&'static str, Features>,
    tools:    ToolPaths,
    index:    Arc<RwLock<Index>>,
    roots:    Vec<PathBuf>,
    swift:    Swift,
}

struct Swift {
    init:      Option<Value>,
    sourcekit: Option<Arc<SourceKit>>,
}

type Delegate = Option<(Arc<SourceKit>, &'static str, Value)>;

fn status(sender: &Sender<Message>, message: &str) {
    let line = message.lines().find(|l| !l.trim().is_empty()).unwrap_or(message);
    let note = Notification::new("magicFormatter/status".into(), json!({ "message": line }));
    sender.send(note.into()).ok();
}

fn respond(sender: &Sender<Message>, id: RequestId, result: Value) {
    sender.send(Response::new_ok(id, result).into()).ok();
}

fn spawn_request(sender: Sender<Message>, id: RequestId, work: impl FnOnce(&Sender<Message>) -> Value + Send + 'static) {
    std::thread::spawn(move || {
        let response = match catch_unwind(AssertUnwindSafe(|| work(&sender))) {
            Ok(result) => Response::new_ok(id, result),
            Err(_) => Response::new_err(id, ErrorCode::InternalError as i32, "magic-formatter: internal error".into()),
        };
        sender.send(response.into()).ok();
    });
}

fn spawn_delegated(
    sender: Sender<Message>,
    id: RequestId,
    delegate: Delegate,
    work: impl FnOnce(&Sender<Message>) -> Value + Send + 'static,
) {
    spawn_request(sender, id, move |sender| {
        delegate.and_then(|(sourcekit, method, params)| sourcekit.request(method, params)).unwrap_or_else(|| work(sender))
    });
}

fn path_string(p: Option<&Value>) -> Option<PathBuf> {
    p.and_then(Value::as_str).filter(|s| !s.is_empty()).map(PathBuf::from)
}

fn full_range(doc: &Document) -> Range {
    let (line, character) = doc.lines.position(&doc.text, doc.text.len());
    Range {
        start: Position {
            line:      0,
            character: 0,
        },
        end:   Position { line, character },
    }
}

fn to_range(text: &str, lines: &LineIndex, (start, end): (usize, usize)) -> Range {
    let (sl, sc) = lines.position(text, start);
    let (el, ec) = lines.position(text, end);
    Range {
        start: Position {
            line:      sl,
            character: sc,
        },
        end:   Position {
            line:      el,
            character: ec,
        },
    }
}

fn parse(lang: &dyn Language, text: &str) -> Option<Tree> {
    let analysis   = lang.analysis()?;
    let mut parser = Parser::new();
    parser.set_language(&analysis.grammar()).ok()?;
    let masked = analysis.mask(text);
    parser.parse(masked.as_deref().unwrap_or(text), None)
}

#[derive(Clone)]
struct OpenFile {
    path: PathBuf,
    uri:  Uri,
    text: String,
    tree: Option<Tree>,
}

impl Server {
    fn sourcekit(&self, lang: &dyn Language) -> Option<Arc<SourceKit>> {
        self.swift.sourcekit.clone().filter(|_| lang.id() == lang_swift::SWIFT.id())
    }

    fn delegate(&self, lang: &dyn Language, method: &'static str, params: Value) -> Delegate {
        Some((self.sourcekit(lang)?, method, params))
    }

    fn start_sourcekit(&mut self, lang: &dyn Language) -> Option<Arc<SourceKit>> {
        let enabled = lang.id() == lang_swift::SWIFT.id() && self.features.get(lang.id()).is_some_and(|f| f.language_server);
        if enabled && let Some(init) = self.swift.init.take() {
            self.swift.sourcekit = SourceKit::spawn(init).map(Arc::new);
        }
        self.swift.sourcekit.clone().filter(|_| enabled)
    }

    fn forward(&mut self, note: &Notification) {
        let document  = &note.params["textDocument"];
        let sourcekit = match note.method.as_str() {
            "textDocument/didOpen" => {
                language_by_id(document["languageId"].as_str().unwrap_or_default()).and_then(|l| self.start_sourcekit(l))
            }
            "textDocument/didChange" | "textDocument/didClose" => {
                document["uri"].as_str().and_then(|u| self.docs.get(u)).and_then(|d| self.sourcekit(d.lang))
            }
            "workspace/didChangeWatchedFiles" => self.swift.sourcekit.clone(),
            _ => None,
        };
        if let Some(sourcekit) = sourcekit {
            sourcekit.notify(&note.method, note.params.clone());
        }
    }

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
            registrations.push(json!({
                "id": "definition",
                "method": "textDocument/definition",
                "registerOptions": { "documentSelector": references },
            }));
            registrations.push(json!({
                "id": "implementation",
                "method": "textDocument/implementation",
                "registerOptions": { "documentSelector": references },
            }));
        }
        let lsp = self.selector(|f| f.language_server);
        if !lsp.is_empty() {
            registrations.extend(features::registrations(&lsp));
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
            "textDocument/definition" => match req.extract::<GotoDefinitionParams>("textDocument/definition") {
                Ok((id, params)) => self.goto(id, params, false),
                Err(e) => self.invalid(id, e),
            },
            "textDocument/implementation" => match req.extract::<GotoDefinitionParams>("textDocument/implementation") {
                Ok((id, params)) => self.goto(id, params, true),
                Err(e) => self.invalid(id, e),
            },
            _ => {
                if let Err(req) = self.feature_request(req) {
                    let resp = Response::new_err(id, ErrorCode::MethodNotFound as i32, format!("unhandled: {}", req.method));
                    self.sender.send(resp.into()).ok();
                }
            }
        }
    }

    fn invalid(&self, id: RequestId, e: impl std::fmt::Debug) {
        let resp = Response::new_err(id, ErrorCode::InvalidParams as i32, format!("{e:?}"));
        self.sender.send(resp.into()).ok();
    }

    fn semantic_tokens(&mut self, id: RequestId, params: SemanticTokensParams) {
        let key  = params.text_document.uri.as_str().to_string();
        let data = self.docs.get(&key).and_then(|doc| {
            let tree                 = doc.tree.as_ref()?;
            let mut spans            = doc.lang.analysis()?.highlighter().spans(tree, &doc.text);
            let enums: HashSet<&str> = enum_declarations(&doc.text).collect();
            let types: HashSet<&str> = type_declarations(&doc.text).collect();
            let index                = self.index.read().unwrap();
            mark_enums(&mut spans, &doc.text, |name| enums.contains(name) || !types.contains(name) && index.is_enum(name));
            mark_functions(&mut spans, &doc.text, |name| !types.contains(name) && !index.is_enum(name) && index.is_function(name));
            if let Some(analysis) = doc.lang.analysis().filter(|_| self.features.get(doc.lang.id()).is_some_and(|f| f.language_server)) {
                refine_locals(analysis, tree, &doc.text, &mut spans);
                analysis.refine(tree, &doc.text, &mut spans);
            }
            Some(encode(&spans, &doc.text, &doc.lines))
        });
        respond(&self.sender, id, json!({ "data": data.unwrap_or_default() }));
    }

    fn formatting(&mut self, id: RequestId, params: DocumentFormattingParams) {
        let key       = params.text_document.uri.as_str().to_string();
        let Some(doc) = self.docs.get(&key) else {
            respond(&self.sender, id, Value::Null);
            return;
        };
        let lang   = doc.lang;
        let text   = doc.text.clone();
        let range  = full_range(doc);
        let dir    = uri_to_path(&params.text_document.uri).and_then(|p| p.parent().map(PathBuf::from));
        let tools  = self.tools.clone();
        let indent = if params.options.insert_spaces {
            " ".repeat(params.options.tab_size as usize)
        } else {
            "\t".into()
        };
        spawn_request(self.sender.clone(), id, move |sender| {
            let ctx = FormatContext {
                dir: dir.as_deref(),
                indent,
                tools: &tools,
            };
            match lang.format(&text, &ctx) {
                Ok(formatted) => {
                    if let Some(w) = &formatted.warning {
                        status(sender, &format!("Magic Formatter ({w})"));
                    }
                    if formatted.text == text {
                        json!([])
                    } else {
                        serde_json::to_value(vec![TextEdit {
                            range,
                            new_text: formatted.text,
                        }])
                        .unwrap()
                    }
                }
                Err(e) => {
                    status(sender, &format!("Magic Formatter: {}", e.lines().find(|l| !l.trim().is_empty()).unwrap_or(&e)));
                    json!([])
                }
            }
        });
    }

    fn references(&mut self, id: RequestId, params: ReferenceParams) {
        let p                                  = params.text_document_position;
        let Some((lang, offset, origin, open)) = self.lookup_context(p.text_document.uri, p.position) else {
            respond(&self.sender, id, Value::Null);
            return;
        };
        let include = params.context.include_declaration;
        let index   = self.index.clone();
        spawn_request(self.sender.clone(), id, move |_| {
            serde_json::to_value(find_references(lang, origin, open, offset, include, &index)).unwrap()
        });
    }

    fn goto(&mut self, id: RequestId, params: GotoDefinitionParams, implementation: bool) {
        let raw                                = serde_json::to_value(&params).unwrap_or_default();
        let p                                  = params.text_document_position_params;
        let Some((lang, offset, origin, open)) = self.lookup_context(p.text_document.uri, p.position) else {
            respond(&self.sender, id, Value::Null);
            return;
        };
        let index  = self.index.clone();
        let roots  = self.roots.clone();
        let method = if implementation {
            "textDocument/implementation"
        } else {
            "textDocument/definition"
        };
        let delegate = self.delegate(lang, method, raw);
        spawn_delegated(self.sender.clone(), id, delegate, move |_| {
            serde_json::to_value(find_definitions(lang, origin, open, offset, implementation, &index, &roots)).unwrap()
        });
    }

    fn lookup_context(&self, uri: Uri, pos: Position) -> Option<(&'static dyn Language, usize, OpenFile, Vec<OpenFile>)> {
        let key    = uri.as_str().to_string();
        let doc    = self.docs.get(&key)?;
        let lang   = doc.lang;
        let offset = doc.lines.offset(&doc.text, pos.line, pos.character);
        let origin = OpenFile {
            path: uri_to_path(&uri).unwrap_or_default(),
            uri,
            text: doc.text.clone(),
            tree: doc.tree.clone(),
        };
        let open: Vec<OpenFile> = self
            .docs
            .iter()
            .filter(|(k, d)| **k != key && d.lang.id() == lang.id())
            .filter_map(|(k, d)| {
                let uri: Uri = k.parse().ok()?;
                Some(OpenFile {
                    path: uri_to_path(&uri)?,
                    uri,
                    text: d.text.clone(),
                    tree: d.tree.clone(),
                })
            })
            .collect();
        Some((lang, offset, origin, open))
    }

    fn handle_notification(&mut self, note: Notification) {
        self.forward(&note);
        match note.method.as_str() {
            "textDocument/didOpen" => {
                let Ok(p) = note.extract::<DidOpenTextDocumentParams>("textDocument/didOpen") else {
                    return;
                };
                let lang = language_by_id(&p.text_document.language_id)
                    .or_else(|| uri_to_path(&p.text_document.uri).and_then(|path| language_for_path(&path)));
                let Some(lang) = lang else { return };
                let mut doc    = Document::new(lang, p.text_document.text);
                if self.needs_tree(lang) {
                    doc.reparse(&mut self.parser);
                }
                self.docs.insert(p.text_document.uri.as_str().to_string(), doc);
                self.publish_diagnostics(&p.text_document.uri);
            }
            "textDocument/didChange" => {
                let Ok(p) = note.extract::<DidChangeTextDocumentParams>("textDocument/didChange") else {
                    return;
                };
                let needs = self.docs.get(p.text_document.uri.as_str()).is_some_and(|d| self.needs_tree(d.lang));
                if let Some(doc) = self.docs.get_mut(p.text_document.uri.as_str()) {
                    doc.apply(p.content_changes);
                    if needs {
                        doc.reparse(&mut self.parser);
                    }
                }
                self.publish_diagnostics(&p.text_document.uri);
            }
            "textDocument/didClose" => {
                let Ok(p) = note.extract::<DidCloseTextDocumentParams>("textDocument/didClose") else {
                    return;
                };
                if let Some(doc) = self.docs.remove(p.text_document.uri.as_str())
                    && self.features.get(doc.lang.id()).is_some_and(|f| f.language_server)
                {
                    features::clear_diagnostics(&self.sender, &p.text_document.uri);
                }
            }
            "workspace/didChangeWatchedFiles" => {
                let Ok(p) = note.extract::<DidChangeWatchedFilesParams>("workspace/didChangeWatchedFiles") else {
                    return;
                };
                let changes: Vec<(PathBuf, bool)> = p
                    .changes
                    .into_iter()
                    .filter_map(|c| Some((uri_to_path(&c.uri)?, c.typ == FileChangeType::DELETED)))
                    .filter(|(path, _)| language_for_path(path).is_some_and(|l| self.features.get(l.id()).is_some_and(|f| f.references)))
                    .collect();
                if changes.is_empty() {
                    return;
                }
                let index = self.index.clone();
                std::thread::spawn(move || {
                    for (path, deleted) in changes {
                        match (deleted, read_source(&path)) {
                            (false, Some(text)) => {
                                let outline = outline(&path, &text);
                                index.write().unwrap().update(&path, &text, outline);
                            }
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
    let Some(analysis) = lang.analysis() else {
        return Vec::new();
    };
    let Some(tree) = origin.tree.clone().or_else(|| parse(lang, &origin.text)) else {
        return Vec::new();
    };
    let Some(symbol) = symbol_at(analysis, &tree, &origin.text, offset) else {
        return Vec::new();
    };

    let locate = |file: &OpenFile, tree: &Tree, out: &mut Vec<Location>| {
        let lines = LineIndex::new(&file.text);
        for hit in occurrences(analysis, tree, &file.text, &symbol, include_declaration) {
            out.push(Location {
                uri:   file.uri.clone(),
                range: to_range(&file.text, &lines, hit),
            });
        }
    };

    if matches!(symbol.target, Target::Local(_)) {
        let mut out = Vec::new();
        locate(&origin, &tree, &mut out);
        return out;
    }
    search(lang, &origin, &tree, &open, &symbol.name, index, locate)
}

fn find_definitions(
    lang: &'static dyn Language,
    origin: OpenFile,
    open: Vec<OpenFile>,
    offset: usize,
    implementation: bool,
    index: &RwLock<Index>,
    roots: &[PathBuf],
) -> Vec<Location> {
    let Some(analysis) = lang.analysis() else {
        return Vec::new();
    };
    let Some(tree) = origin.tree.clone().or_else(|| parse(lang, &origin.text)) else {
        return Vec::new();
    };
    let imported = analysis.import_definitions(&tree, &origin.text, offset, &origin.path, roots);
    if !imported.is_empty() {
        return imported
            .into_iter()
            .filter_map(|(path, start, end)| {
                let text = match open.iter().chain(std::iter::once(&origin)).find(|f| f.path == path) {
                    Some(f) => f.text.clone(),
                    None => read_source(&path)?,
                };
                let lines = LineIndex::new(&text);
                Some(Location {
                    uri:   path_to_uri(&path)?,
                    range: to_range(&text, &lines, (start, end)),
                })
            })
            .collect();
    }
    let Some(symbol) = symbol_at(analysis, &tree, &origin.text, offset) else {
        return Vec::new();
    };
    let Some(node) = identifier_at(analysis, &tree, offset) else {
        return Vec::new();
    };
    let kinds = match analysis.implementation_kinds() {
        k if implementation && !k.is_empty() => k,
        _ => analysis.definition_kinds(node),
    };
    if let Target::Local(def) = &symbol.target {
        let lines = LineIndex::new(&origin.text);
        return vec![Location {
            uri:   origin.uri.clone(),
            range: to_range(&origin.text, &lines, (def.start, def.end)),
        }];
    }
    let local = definition_hits(analysis, &tree, &origin.text, &symbol.name, kinds, implementation);
    if !implementation && !local.is_empty() {
        let lines = LineIndex::new(&origin.text);
        return local
            .into_iter()
            .map(|hit| Location {
                uri:   origin.uri.clone(),
                range: to_range(&origin.text, &lines, hit),
            })
            .collect();
    }
    let mut out = search(lang, &origin, &tree, &open, &symbol.name, index, |file, tree, out| {
        let lines = LineIndex::new(&file.text);
        for hit in definition_hits(analysis, tree, &file.text, &symbol.name, kinds, implementation) {
            out.push(Location {
                uri:   file.uri.clone(),
                range: to_range(&file.text, &lines, hit),
            });
        }
    });
    if !implementation && out.iter().any(|l| l.uri == origin.uri) {
        out.retain(|l| l.uri == origin.uri);
    }
    if out.is_empty()
        && let Some(range) = analysis.import_fallback(&tree, &origin.text, &symbol.name)
    {
        let lines = LineIndex::new(&origin.text);
        out.push(Location {
            uri:   origin.uri.clone(),
            range: to_range(&origin.text, &lines, range),
        });
    }
    out
}

fn definition_hits(
    analysis: &dyn Analysis,
    tree: &Tree,
    src: &str,
    name: &str,
    kinds: &[(&str, &str)],
    implementation: bool,
) -> Vec<(usize, usize)> {
    if implementation && let Some(hits) = analysis.implementations(tree, src, name) {
        return hits;
    }
    let root = tree.root_node();
    definitions::find(tree, src, name, kinds)
        .into_iter()
        .filter(|&(start, end)| root.descendant_for_byte_range(start, end).is_some_and(|n| analysis.is_definition(n)))
        .collect()
}

fn search(
    lang: &'static dyn Language,
    origin: &OpenFile,
    tree: &Tree,
    open: &[OpenFile],
    name: &str,
    index: &RwLock<Index>,
    locate: impl Fn(&OpenFile, &Tree, &mut Vec<Location>),
) -> Vec<Location> {
    let mut out = Vec::new();
    locate(origin, tree, &mut out);

    let mut seen: Vec<PathBuf> = vec![origin.path.clone()];
    for file in open {
        if file.text.contains(name)
            && let Some(tree) = file.tree.clone().or_else(|| parse(lang, &file.text))
        {
            locate(file, &tree, &mut out);
        }
        seen.push(file.path.clone());
    }

    let candidates = index.read().unwrap().files_with(name);
    for path in candidates {
        if seen.contains(&path) || language_for_path(&path).is_none_or(|l| l.id() != lang.id()) {
            continue;
        }
        let Some(text) = read_source(&path) else { continue };
        if !text.contains(name) {
            continue;
        }
        let Some(uri)  = path_to_uri(&path) else { continue };
        let Some(tree) = parse(lang, &text) else { continue };
        locate(
            &OpenFile {
                path,
                uri,
                text,
                tree: None,
            },
            &tree,
            &mut out,
        );
    }
    out
}

pub fn run() {
    let (connection, io_threads) = Connection::stdio();
    let Ok((id, params))         = connection.initialize_start() else { return };
    let init: InitializeParams   = serde_json::from_value(params.clone()).unwrap_or_default();
    let options                  = init.initialization_options.unwrap_or(Value::Null);

    let mut sourcekit_init = params;
    if let Some(p) = sourcekit_init.as_object_mut() {
        p.remove("initializationOptions");
    }

    let mut features = HashMap::new();
    for lang in LANGUAGES.iter().filter(|l| l.analysis().is_some()) {
        let cfg        = &options["languages"][lang.id()];
        let references = cfg["references"].as_bool().unwrap_or(true);
        features.insert(
            lang.id(),
            Features {
                semantic_tokens: cfg["semanticTokens"].as_bool().unwrap_or(true),
                references,
                language_server: references && lang.analysis().is_some_and(|a| a.language_server()),
            },
        );
    }
    let tools = ToolPaths {
        rustfmt: path_string(options.get("rustfmtPath")),
        topcoat: path_string(options.get("topcoatPath")),
    };

    let capabilities = json!({
        "capabilities": { "textDocumentSync": { "openClose": true, "change": 2 } },
        "serverInfo": { "name": "magic-formatter", "version": env!("CARGO_PKG_VERSION") },
    });
    if connection.initialize_finish(id, capabilities).is_err() {
        return;
    }

    let roots: Vec<PathBuf> = init.workspace_folders.unwrap_or_default().iter().filter_map(|f| uri_to_path(&f.uri)).collect();
    let mut server          = Server {
        sender: connection.sender.clone(),
        docs: HashMap::new(),
        parser: Parser::new(),
        features,
        tools,
        index: Arc::new(RwLock::new(Index::default())),
        roots: roots.clone(),
        swift: Swift {
            init:      options["sourcekitLsp"].as_bool().unwrap_or(true).then_some(sourcekit_init),
            sourcekit: None,
        },
    };
    server.register_capabilities();

    let languages: Vec<&'static str> =
        LANGUAGES.iter().filter(|l| server.features.get(l.id()).is_some_and(|f| f.references)).map(|l| l.id()).collect();
    index_roots(roots, languages, server.index.clone());

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
