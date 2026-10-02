use std::collections::HashMap;

use magic_core::describe::{Call, Description, collapse_ws, comments_before, escape_markdown, first_line};
use magic_core::highlight::{Kind, Span, token_type};
use magic_core::symbols::{SymbolInfo, SymbolKind};
use tree_sitter::{Node, Tree};

pub const BUILTINS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "__import__",
    "self",
    "cls",
    "__name__",
    "__file__",
    "NotImplemented",
    "Ellipsis",
    "Exception",
    "BaseException",
    "ValueError",
    "TypeError",
    "KeyError",
    "IndexError",
    "AttributeError",
    "RuntimeError",
    "StopIteration",
    "NotImplementedError",
    "OSError",
    "ImportError",
    "AssertionError",
];

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    &src[node.start_byte()..node.end_byte()]
}

fn owner(node: Node) -> Option<Node> {
    node.parent().and_then(|p| if p.kind() == "decorated_definition" { p.parent() } else { Some(p) })
}

pub fn enclosing_class(node: Node) -> Option<Node> {
    owner(node).filter(|b| b.kind() == "block").and_then(|b| b.parent()).filter(|c| c.kind() == "class_definition")
}

fn outer(node: Node) -> Node {
    node.parent().filter(|p| p.kind() == "decorated_definition").unwrap_or(node)
}

fn is_constant(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn function_signature(def: Node, src: &str) -> String {
    let end     = def.child_by_field_name("parameters").map_or(def.end_byte(), |p| p.end_byte());
    let mut sig = collapse_ws(&src[def.start_byte()..end]);
    if let Some(ret) = def.child_by_field_name("return_type") {
        sig.push_str(" -> ");
        sig.push_str(&collapse_ws(text(ret, src)));
    }
    sig
}

fn parameters(def: Node, src: &str) -> Vec<String> {
    let Some(params) = def.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = params.walk();
    let mut out: Vec<String> = params
        .named_children(&mut cursor)
        .filter(|p| !matches!(p.kind(), "keyword_separator" | "positional_separator" | "comment"))
        .map(|p| collapse_ws(text(p, src)))
        .collect();
    let bound = enclosing_class(def).is_some()
        && out.first().is_some_and(|p| p == "self" || p == "cls" || p.starts_with("self:") || p.starts_with("cls:"));
    if bound {
        out.remove(0);
    }
    out
}

fn docstring(def: Node, src: &str) -> Option<String> {
    let body       = def.child_by_field_name("body")?;
    let mut cursor = body.walk();
    let first      = body.named_children(&mut cursor).find(|c| c.kind() != "comment")?;
    if first.kind() != "expression_statement" || first.named_child_count() != 1 {
        return None;
    }
    let string           = first.named_child(0).filter(|s| s.kind() == "string")?;
    let raw              = text(string, src).trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let quote            = ["\"\"\"", "'''", "\"", "'"].into_iter().find(|q| raw.starts_with(q))?;
    let inner            = raw.strip_prefix(quote)?.strip_suffix(quote).unwrap_or(raw);
    let lines: Vec<&str> = inner.lines().collect();
    let indent           = lines.iter().skip(1).filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let mut out: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                l.trim().to_string()
            } else {
                l.get(indent..).unwrap_or(l.trim_start()).trim_end().to_string()
            }
        })
        .collect();
    while out.first().is_some_and(|l| l.is_empty()) {
        out.remove(0);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    (!out.is_empty()).then(|| escape_markdown(&out.join("\n")))
}

fn docs(def: Node, src: &str) -> Option<String> {
    docstring(def, src).or_else(|| comments_before(outer(def), src, &["comment"]).map(|c| escape_markdown(&c)))
}

fn init_parameters(class: Node, src: &str) -> Vec<String> {
    let Some(body) = class.child_by_field_name("body") else {
        return Vec::new();
    };
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .map(|c| {
            if c.kind() == "decorated_definition" {
                c.child_by_field_name("definition").unwrap_or(c)
            } else {
                c
            }
        })
        .find(|c| c.kind() == "function_definition" && c.child_by_field_name("name").is_some_and(|n| text(n, src) == "__init__"))
        .map(|init| parameters(init, src))
        .unwrap_or_default()
}

pub fn symbol(node: Node, src: &str) -> Option<SymbolInfo> {
    let info = |name: Node, kind, detail| SymbolInfo {
        name: text(name, src).to_string(),
        kind,
        selection: (name.start_byte(), name.end_byte()),
        detail,
    };
    match node.kind() {
        "function_definition" => {
            let name = node.child_by_field_name("name")?;
            let kind = match enclosing_class(node) {
                Some(_) if text(name, src) == "__init__" => SymbolKind::Constructor,
                Some(_) => SymbolKind::Method,
                None => SymbolKind::Function,
            };
            let detail = node.child_by_field_name("parameters").map(|p| collapse_ws(text(p, src)));
            Some(info(name, kind, detail))
        }
        "class_definition" => Some(info(node.child_by_field_name("name")?, SymbolKind::Class, None)),
        "assignment" => {
            let stmt  = node.parent().filter(|p| p.kind() == "expression_statement")?;
            let scope = stmt.parent()?;
            let left  = node.child_by_field_name("left").filter(|l| l.kind() == "identifier")?;
            let kind  = match scope.kind() {
                "module" if is_constant(text(left, src)) => SymbolKind::Constant,
                "module" => SymbolKind::Variable,
                "block" if scope.parent().is_some_and(|c| c.kind() == "class_definition") => SymbolKind::Property,
                _ => return None,
            };
            let detail = node.child_by_field_name("type").map(|t| collapse_ws(text(t, src)));
            Some(info(left, kind, detail))
        }
        _ => None,
    }
}

pub fn describe(node: Node, src: &str) -> Option<Description> {
    let parent = node.parent()?;
    let simple = |signature: String, docs: Option<String>| Description {
        signature,
        docs,
        params: Vec::new(),
    };
    match parent.kind() {
        "function_definition" if parent.child_by_field_name("name") == Some(node) => Some(Description {
            signature: function_signature(parent, src),
            docs:      docs(parent, src),
            params:    parameters(parent, src),
        }),
        "class_definition" if parent.child_by_field_name("name") == Some(node) => {
            let end = parent.child_by_field_name("superclasses").unwrap_or(node).end_byte();
            Some(Description {
                signature: collapse_ws(&src[parent.start_byte()..end]),
                docs:      docs(parent, src),
                params:    init_parameters(parent, src),
            })
        }
        "parameters" | "lambda_parameters" => Some(simple(format!("(parameter) {}", text(node, src)), None)),
        "typed_parameter" | "default_parameter" | "typed_default_parameter" | "list_splat_pattern" | "dictionary_splat_pattern"
            if parent.parent().is_some_and(|p| matches!(p.kind(), "parameters" | "lambda_parameters" | "typed_parameter")) =>
        {
            let param = if parent.parent().is_some_and(|p| p.kind() == "typed_parameter") {
                parent.parent()?
            } else {
                parent
            };
            Some(simple(format!("(parameter) {}", collapse_ws(text(param, src))), None))
        }
        _ => {
            let assignment = std::iter::successors(Some(node), |n| n.parent())
                .take_while(|n| !matches!(n.kind(), "block" | "module"))
                .find(|n| {
                    matches!(n.kind(), "assignment" | "augmented_assignment")
                        && n.child_by_field_name("left")
                            .is_some_and(|l| l.start_byte() <= node.start_byte() && node.end_byte() <= l.end_byte())
                });
            if let Some(a) = assignment {
                let stmt = a.parent().filter(|p| p.kind() == "expression_statement").unwrap_or(a);
                let docs = comments_before(stmt, src, &["comment"]).map(|c| escape_markdown(&c));
                return Some(simple(first_line(text(a, src), 120), docs));
            }
            let binding =
                std::iter::successors(Some(node), |n| n.parent()).take_while(|n| !matches!(n.kind(), "block" | "module")).find(|n| {
                    matches!(
                        n.kind(),
                        "import_statement"
                            | "import_from_statement"
                            | "as_pattern_target"
                            | "for_statement"
                            | "for_in_clause"
                            | "named_expression"
                            | "global_statement"
                            | "nonlocal_statement"
                    )
                })?;
            let within = |field: &str| {
                binding.child_by_field_name(field).is_some_and(|f| f.start_byte() <= node.start_byte() && node.end_byte() <= f.end_byte())
            };
            match binding.kind() {
                "import_statement" | "import_from_statement" | "global_statement" | "nonlocal_statement" => {
                    Some(simple(first_line(&collapse_ws(text(binding, src)), 120), None))
                }
                "for_statement" | "for_in_clause" if within("left") => Some(simple(format!("(variable) {}", text(node, src)), None)),
                "named_expression" if within("name") => Some(simple(format!("(variable) {}", text(node, src)), None)),
                "as_pattern_target" => Some(simple(format!("(variable) {}", text(node, src)), None)),
                _ => None,
            }
        }
    }
}

pub fn call_at<'t>(node: Node<'t>, offset: usize) -> Option<Call<'t>> {
    let args = std::iter::successors(Some(node), |n| n.parent()).find(|n| {
        n.kind() == "argument_list"
            && n.parent().is_some_and(|p| p.kind() == "call")
            && n.start_byte() < offset
            && (offset < n.end_byte() || n.child(n.child_count().wrapping_sub(1) as _).is_none_or(|c| c.kind() != ")" || c.is_missing()))
    })?;
    let function = args.parent()?.child_by_field_name("function")?;
    let callee   = match function.kind() {
        "identifier" => function,
        "attribute" => function.child_by_field_name("attribute")?,
        _ => return None,
    };
    Some(Call { callee, args })
}

pub fn fold(node: Node) -> Option<(usize, usize)> {
    match node.kind() {
        "block" => Some((node.parent()?.start_byte(), node.end_byte())),
        "string" if node.start_position().row < node.end_position().row => Some((node.start_byte(), node.end_byte())),
        _ => None,
    }
}

fn imports<'s>(node: Node, src: &'s str, modules: &mut Vec<&'s str>, imported: &mut Vec<usize>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let from = child.kind() == "import_from_statement";
        if !from && child.kind() != "import_statement" {
            imports(child, src, modules, imported);
            continue;
        }
        let mut c = child.walk();
        for item in child.children_by_field_name("name", &mut c) {
            let bound = match item.kind() {
                "aliased_import" => item.child_by_field_name("alias"),
                _ => item.named_child(0),
            };
            match (bound, from) {
                (Some(b), true) => imported.push(b.start_byte()),
                (Some(b), false) => modules.push(text(b, src)),
                _ => {}
            }
        }
    }
}

pub fn refine(tree: &Tree, src: &str, spans: &mut [Span]) {
    let (variable, namespace) = (token_type("variable"), token_type("namespace"));
    let adoptable             = ["function", "type", "decorator", "namespace"].map(token_type);
    let mut modules           = Vec::new();
    let mut imported          = Vec::new();
    imports(tree.root_node(), src, &mut modules, &mut imported);
    let mut usage: HashMap<&str, Kind> = HashMap::new();
    for span in spans.iter().filter(|s| adoptable.contains(&s.kind.ty)) {
        usage.entry(&src[span.start..span.end]).or_insert(span.kind);
    }
    for span in spans.iter_mut().filter(|s| s.kind.ty == variable && s.kind.mods == 0) {
        let name = &src[span.start..span.end];
        if modules.contains(&name) {
            span.kind.ty = namespace;
        } else if imported.contains(&span.start)
            && let Some(kind) = usage.get(name)
        {
            span.kind = *kind;
        }
    }
}

fn members(node: Node, class: Node, src: &str, out: &mut Vec<(String, SymbolKind)>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "class_definition" => continue,
            "function_definition" if enclosing_class(child) == Some(class) => {
                if let Some(name) = child.child_by_field_name("name") {
                    out.push((text(name, src).to_string(), SymbolKind::Method));
                }
            }
            "attribute" => {
                let target = child.child_by_field_name("object").is_some_and(|o| text(o, src) == "self");
                let stored = node.kind() == "assignment" && node.child_by_field_name("left") == Some(child);
                if target
                    && stored
                    && let Some(attr) = child.child_by_field_name("attribute")
                {
                    out.push((text(attr, src).to_string(), SymbolKind::Property));
                }
            }
            "assignment" if node.parent().and_then(|b| b.parent()) == Some(class) => {
                if let Some(left) = child.child_by_field_name("left").filter(|l| l.kind() == "identifier") {
                    out.push((text(left, src).to_string(), SymbolKind::Property));
                }
            }
            _ => {}
        }
        members(child, class, src, out);
    }
}

pub fn self_members(node: Node, src: &str) -> Vec<(String, SymbolKind)> {
    let Some(class) = std::iter::successors(Some(node), |n| n.parent()).find(|n| n.kind() == "class_definition") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(body) = class.child_by_field_name("body") {
        members(body, class, src, &mut out);
    }
    out
}
