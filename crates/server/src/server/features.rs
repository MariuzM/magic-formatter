use std::collections::HashMap;
use std::path::Path;

use magic_core::Analysis;
use magic_core::completion::{self, Item, ItemKind, is_member_access};
use magic_core::describe::{Description, active_parameter, markdown};
use magic_core::diagnostics::syntax_errors;
use magic_core::folding::{FoldKind, folding_ranges, selection_ranges};
use magic_core::symbols::{Symbol, SymbolKind, document_symbols};
use magic_core::text::utf16_len;

use super::*;

pub fn registrations(selector: &[Value]) -> Vec<Value> {
    let methods = [
        ("hover", "textDocument/hover", json!({})),
        ("documentSymbol", "textDocument/documentSymbol", json!({})),
        ("documentHighlight", "textDocument/documentHighlight", json!({})),
        ("rename", "textDocument/rename", json!({ "prepareProvider": true })),
        ("foldingRange", "textDocument/foldingRange", json!({})),
        ("selectionRange", "textDocument/selectionRange", json!({})),
        ("completion", "textDocument/completion", json!({ "triggerCharacters": ["."] })),
        ("signatureHelp", "textDocument/signatureHelp", json!({ "triggerCharacters": ["(", ","], "retriggerCharacters": [","] })),
    ];
    let mut out: Vec<Value> = methods
        .into_iter()
        .map(|(id, method, mut options)| {
            options["documentSelector"] = json!(selector);
            json!({ "id": id, "method": method, "registerOptions": options })
        })
        .collect();
    out.push(json!({ "id": "workspaceSymbol", "method": "workspace/symbol", "registerOptions": {} }));
    out
}

pub fn clear_diagnostics(sender: &Sender<Message>, uri: &Uri) {
    let note = Notification::new("textDocument/publishDiagnostics".into(), json!({ "uri": uri.as_str(), "diagnostics": [] }));
    sender.send(note.into()).ok();
}

fn uri_of(params: &Value) -> Option<Uri> {
    params["textDocument"]["uri"].as_str()?.parse().ok()
}

fn position_of(params: &Value) -> Option<Position> {
    serde_json::from_value(params["position"].clone()).ok()
}

fn markup(value: String) -> Value {
    json!({ "kind": "markdown", "value": value })
}

fn completion_kind(kind: ItemKind) -> u8 {
    match kind {
        ItemKind::Keyword => 14,
        ItemKind::Builtin => 3,
        ItemKind::Text => 1,
        ItemKind::Symbol(k) => match k {
            SymbolKind::Module | SymbolKind::Namespace => 9,
            SymbolKind::Class => 7,
            SymbolKind::Method => 2,
            SymbolKind::Property => 10,
            SymbolKind::Field => 5,
            SymbolKind::Constructor => 4,
            SymbolKind::Enum => 13,
            SymbolKind::Interface => 8,
            SymbolKind::Function => 3,
            SymbolKind::Variable => 6,
            SymbolKind::Constant => 21,
            SymbolKind::EnumMember => 20,
            SymbolKind::Struct => 22,
            SymbolKind::TypeParameter => 25,
        },
    }
}

fn item_json(item: Item) -> Value {
    let sort = match item.kind {
        _ if item.label.starts_with('_') => "4",
        ItemKind::Symbol(_) => "1",
        ItemKind::Keyword | ItemKind::Builtin => "2",
        ItemKind::Text => "3",
    };
    let mut value = json!({ "label": item.label, "kind": completion_kind(item.kind), "sortText": format!("{sort}{}", item.label) });
    if let Some(detail) = item.detail {
        value["detail"] = json!(detail);
    }
    value
}

fn symbol_json(symbol: &Symbol, text: &str, lines: &LineIndex) -> Value {
    let children: Vec<Value> = symbol.children.iter().filter(|c| !c.name.is_empty()).map(|c| symbol_json(c, text, lines)).collect();
    let mut value            = json!({
        "name": symbol.name,
        "kind": symbol.kind as u8,
        "range": to_range(text, lines, symbol.range),
        "selectionRange": to_range(text, lines, symbol.selection),
        "children": children,
    });
    if let Some(detail) = &symbol.detail {
        value["detail"] = json!(detail);
    }
    value
}

fn fuzzy(name: &str, query: &str) -> bool {
    let mut chars = name.chars().flat_map(char::to_lowercase);
    query.chars().flat_map(char::to_lowercase).all(|q| chars.any(|c| c == q))
}

fn describe_at(
    lang: &'static dyn Language,
    origin: OpenFile,
    open: Vec<OpenFile>,
    offset: usize,
    index: &RwLock<Index>,
    roots: &[PathBuf],
) -> Option<(Description, (usize, usize))> {
    let analysis = lang.analysis()?;
    let tree     = origin.tree.clone().or_else(|| parse(lang, &origin.text))?;
    let node     = identifier_at(analysis, &tree, offset)?;
    let range    = (node.start_byte(), node.end_byte());
    let name     = node.utf8_text(origin.text.as_bytes()).ok()?.to_string();
    if let Some(d) = analysis.describe(node, &origin.text) {
        return Some((d, range));
    }
    if analysis.builtins().contains(&name.as_str()) {
        return None;
    }
    let (uri, text) = (origin.uri.clone(), origin.text.clone());
    let target      = find_definitions(lang, origin, open, offset, false, index, roots).into_iter().next()?;
    let start       = target.range.start;
    if target.uri != uri && start == target.range.end && start.line == 0 && start.character == 0 {
        let signature = format!("(module) {name}");
        return Some((
            Description {
                signature,
                docs: None,
                params: Vec::new(),
            },
            range,
        ));
    }
    let (text, tree) = if target.uri == uri {
        (text, tree)
    } else {
        parsed(lang, &uri_to_path(&target.uri)?)?
    };
    let lines = LineIndex::new(&text);
    let def   = identifier_at(analysis, &tree, lines.offset(&text, start.line, start.character))?;
    analysis.describe(def, &text).map(|d| (d, range))
}

impl Server {
    fn language_server(&self, lang: &dyn Language) -> bool {
        self.features.get(lang.id()).is_some_and(|f| f.language_server)
    }

    fn doc(&self, params: &Value) -> Option<(&Document, &'static dyn Analysis, &Tree)> {
        let doc = self.docs.get(uri_of(params)?.as_str())?;
        if !self.language_server(doc.lang) {
            return None;
        }
        Some((doc, doc.lang.analysis()?, doc.tree.as_ref()?))
    }

    fn context(&self, params: &Value) -> Option<(&'static dyn Language, usize, OpenFile, Vec<OpenFile>)> {
        let (lang, offset, origin, open) = self.lookup_context(uri_of(params)?, position_of(params)?)?;
        self.language_server(lang).then_some((lang, offset, origin, open))
    }

    pub(super) fn publish_diagnostics(&self, uri: &Uri) {
        let Some(doc)  = self.docs.get(uri.as_str()) else { return };
        let enabled    = doc.lang.analysis().is_some_and(|a| a.syntax_diagnostics()) && self.language_server(doc.lang);
        let Some(tree) = doc.tree.as_ref().filter(|_| enabled) else {
            return;
        };
        let diagnostics: Vec<Value> = syntax_errors(tree)
            .into_iter()
            .map(|(start, end, message)| {
                let range = to_range(&doc.text, &doc.lines, (start, end));
                json!({ "range": range, "severity": 1, "source": "magic-formatter", "message": message })
            })
            .collect();
        let note = Notification::new(
            "textDocument/publishDiagnostics".into(),
            json!({ "uri": uri.as_str(), "diagnostics": diagnostics }),
        );
        self.sender.send(note.into()).ok();
    }

    pub(super) fn feature_request(&mut self, req: Request) -> Result<(), Request> {
        let (id, params) = (req.id.clone(), &req.params);
        match req.method.as_str() {
            "textDocument/hover" => self.hover(id, params),
            "textDocument/signatureHelp" => self.signature_help(id, params),
            "textDocument/documentSymbol" => self.document_symbol(id, params),
            "workspace/symbol" => self.workspace_symbol(id, params),
            "textDocument/documentHighlight" => self.document_highlight(id, params),
            "textDocument/prepareRename" => self.prepare_rename(id, params),
            "textDocument/rename" => self.rename(id, params),
            "textDocument/foldingRange" => self.folding_range(id, params),
            "textDocument/selectionRange" => self.selection_range(id, params),
            "textDocument/completion" => self.completion(id, params),
            _ => return Err(req),
        }
        Ok(())
    }

    fn hover(&self, id: RequestId, params: &Value) {
        let Some((lang, offset, origin, open)) = self.context(params) else {
            return respond(&self.sender, id, Value::Null);
        };
        let (index, roots) = (self.index.clone(), self.roots.clone());
        let delegate       = self.delegate(lang, "textDocument/hover", params.clone());
        spawn_delegated(&self.pool, id, delegate, move |_| {
            let fence = lang.analysis().map_or("", |a| a.fence());
            let text  = origin.text.clone();
            match describe_at(lang, origin, open, offset, &index, &roots) {
                Some((d, range)) => {
                    let lines = LineIndex::new(&text);
                    json!({ "contents": markup(markdown(fence, &d)), "range": to_range(&text, &lines, range) })
                }
                None => Value::Null,
            }
        });
    }

    fn signature_help(&self, id: RequestId, params: &Value) {
        let Some((lang, offset, origin, open)) = self.context(params) else {
            return respond(&self.sender, id, Value::Null);
        };
        let (index, roots) = (self.index.clone(), self.roots.clone());
        let delegate       = self.delegate(lang, "textDocument/signatureHelp", params.clone());
        spawn_delegated(&self.pool, id, delegate, move |_| {
            let Some(analysis) = lang.analysis() else { return Value::Null };
            let Some(tree)     = origin.tree.clone() else { return Value::Null };
            let call           = tree.root_node().descendant_for_byte_range(offset, offset).and_then(|n| analysis.call_at(n, offset));
            let Some(call)     = call else { return Value::Null };
            let active         = active_parameter(call.args, offset);
            let callee         = call.callee.start_byte();
            let name           = call.callee.utf8_text(origin.text.as_bytes()).unwrap_or_default().to_string();
            let Some((d, _))   = describe_at(lang, origin, open, callee, &index, &roots) else {
                return Value::Null;
            };
            let mut label      = format!("{name}(");
            let mut parameters = Vec::new();
            for (i, p) in d.params.iter().enumerate() {
                if i > 0 {
                    label.push_str(", ");
                }
                let start = utf16_len(&label);
                label.push_str(p);
                parameters.push(json!({ "label": [start, utf16_len(&label)] }));
            }
            label.push(')');
            let mut signature = json!({ "label": label, "parameters": parameters });
            if let Some(docs) = d.docs {
                signature["documentation"] = markup(docs);
            }
            json!({ "signatures": [signature], "activeSignature": 0, "activeParameter": active.min(d.params.len().saturating_sub(1)) })
        });
    }

    fn document_symbol(&self, id: RequestId, params: &Value) {
        let result = self.doc(params).map(|(doc, analysis, tree)| {
            let symbols = document_symbols(analysis, tree, &doc.text);
            symbols.iter().filter(|s| !s.name.is_empty()).map(|s| symbol_json(s, &doc.text, &doc.lines)).collect::<Vec<_>>()
        });
        respond(&self.sender, id, json!(result));
    }

    fn workspace_symbol(&self, id: RequestId, params: &Value) {
        let query                     = params["query"].as_str().unwrap_or_default().to_string();
        let index                     = self.index.clone();
        let wanted: Vec<&'static str> = LANGUAGES.iter().filter(|l| self.language_server(**l)).map(|l| l.id()).collect();
        spawn_request(&self.pool, id, move |_| {
            let index = index.read().unwrap();
            let hits: Vec<Value> = index
                .outline()
                .filter(|(path, s)| fuzzy(&s.name, &query) && language_for_path(path).is_some_and(|l| wanted.contains(&l.id())))
                .take(500)
                .filter_map(|(path, s)| {
                    let start     = json!({ "line": s.line, "character": s.start });
                    let end       = json!({ "line": s.line, "character": s.end });
                    let location  = json!({ "uri": path_to_uri(path)?.as_str(), "range": { "start": start, "end": end } });
                    let mut value = json!({ "name": &*s.name, "kind": s.kind as u8, "location": location });
                    if let Some(container) = &s.container {
                        value["containerName"] = json!(&**container);
                    }
                    Some(value)
                })
                .collect();
            json!(hits)
        });
    }

    fn document_highlight(&self, id: RequestId, params: &Value) {
        let result = self.doc(params).and_then(|(doc, analysis, tree)| {
            let pos    = position_of(params)?;
            let offset = doc.lines.offset(&doc.text, pos.line, pos.character);
            let symbol = symbol_at(analysis, tree, &doc.text, offset)?;
            let def    = match &symbol.target {
                Target::Local(def) => Some(def.start),
                Target::Global => None,
            };
            let hits: Vec<Value> = occurrences(analysis, tree, &doc.text, &symbol, true)
                .into_iter()
                .map(|hit| json!({ "range": to_range(&doc.text, &doc.lines, hit), "kind": if Some(hit.0) == def { 3 } else { 2 } }))
                .collect();
            Some(hits)
        });
        respond(&self.sender, id, json!(result));
    }

    fn prepare_rename(&self, id: RequestId, params: &Value) {
        let result = self.doc(params).and_then(|(doc, analysis, tree)| {
            let pos    = position_of(params)?;
            let offset = doc.lines.offset(&doc.text, pos.line, pos.character);
            let node   = identifier_at(analysis, tree, offset)?;
            let name   = node.utf8_text(doc.text.as_bytes()).ok()?;
            if analysis.builtins().contains(&name) {
                return None;
            }
            Some(json!({ "range": to_range(&doc.text, &doc.lines, (node.start_byte(), node.end_byte())), "placeholder": name }))
        });
        respond(&self.sender, id, json!(result));
    }

    fn rename(&self, id: RequestId, params: &Value) {
        let new_name = params["newName"].as_str().unwrap_or_default().to_string();
        let valid = new_name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
            && new_name.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !valid {
            let resp = Response::new_err(id, ErrorCode::InvalidParams as i32, format!("`{new_name}` is not a valid identifier"));
            self.sender.send(resp.into()).ok();
            return;
        }
        let Some((lang, offset, origin, open)) = self.context(params) else {
            return respond(&self.sender, id, Value::Null);
        };
        let index = self.index.clone();
        spawn_request(&self.pool, id, move |_| {
            let mut changes: HashMap<String, Vec<Value>> = HashMap::new();
            for loc in find_references(lang, origin, open, offset, true, &index) {
                changes.entry(loc.uri.as_str().to_string()).or_default().push(json!({ "range": loc.range, "newText": new_name }));
            }
            json!({ "changes": changes })
        });
    }

    fn folding_range(&self, id: RequestId, params: &Value) {
        let result = self.doc(params).map(|(doc, analysis, tree)| {
            folding_ranges(analysis, tree, &doc.text)
                .into_iter()
                .map(|(start, end, kind)| {
                    let mut value = json!({ "startLine": start, "endLine": end });
                    match kind {
                        FoldKind::Comment => value["kind"] = json!("comment"),
                        FoldKind::Imports => value["kind"] = json!("imports"),
                        FoldKind::Region => {}
                    }
                    value
                })
                .collect::<Vec<_>>()
        });
        respond(&self.sender, id, json!(result));
    }

    fn selection_range(&self, id: RequestId, params: &Value) {
        let result = self.doc(params).map(|(doc, _, tree)| {
            let positions: Vec<Position> = serde_json::from_value(params["positions"].clone()).unwrap_or_default();
            positions
                .into_iter()
                .map(|pos| {
                    let offset                   = doc.lines.offset(&doc.text, pos.line, pos.character);
                    let mut value: Option<Value> = None;
                    for range in selection_ranges(tree, offset).into_iter().rev() {
                        let mut node = json!({ "range": to_range(&doc.text, &doc.lines, range) });
                        if let Some(parent) = value.take() {
                            node["parent"] = parent;
                        }
                        value = Some(node);
                    }
                    value.unwrap_or_else(|| json!({ "range": { "start": pos, "end": pos } }))
                })
                .collect::<Vec<_>>()
        });
        respond(&self.sender, id, json!(result));
    }

    fn completion(&self, id: RequestId, params: &Value) {
        let Some((lang, offset, origin, _)) = self.context(params) else {
            return respond(&self.sender, id, Value::Null);
        };
        let (index, roots) = (self.index.clone(), self.roots.clone());
        let delegate       = self.delegate(lang, "textDocument/completion", params.clone());
        spawn_delegated(&self.pool, id, delegate, move |_| {
            let Some(analysis) = lang.analysis() else { return Value::Null };
            let Some(tree)     = origin.tree.clone().or_else(|| parse(lang, &origin.text)) else {
                return Value::Null;
            };
            let text  = &origin.text;
            let items = if is_member_access(text, offset) {
                match analysis.member_completions(&tree, text, offset, &origin.path, &roots) {
                    Some(list) if !list.is_empty() => list
                        .into_iter()
                        .map(|(label, kind)| Item {
                            label,
                            kind: ItemKind::Symbol(kind),
                            detail: None,
                        })
                        .collect(),
                    _ => completion::members(analysis, &tree, text, offset, workspace_items(&index, lang, true, &origin.path)),
                }
            } else {
                completion::complete(analysis, &tree, text, offset, workspace_items(&index, lang, false, &origin.path))
            };
            json!({ "isIncomplete": false, "items": items.into_iter().map(item_json).collect::<Vec<_>>() })
        });
    }
}

fn workspace_items(index: &RwLock<Index>, lang: &dyn Language, members: bool, exclude: &Path) -> Vec<Item> {
    let index = index.read().unwrap();
    index
        .outline()
        .filter(|(path, s)| {
            *path != exclude
                && (if members {
                    s.kind.is_member()
                } else {
                    !s.kind.is_member() && s.container.is_none()
                })
                && language_for_path(path).is_some_and(|l| l.id() == lang.id())
        })
        .map(|(path, s)| Item {
            label:  s.name.to_string(),
            kind:   ItemKind::Symbol(s.kind),
            detail: path.file_name().map(|f| f.to_string_lossy().into_owned()),
        })
        .collect()
}
