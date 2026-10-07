use std::collections::HashMap;

use magic_core::cursor::SpanCursor;
use magic_core::describe::{Call, Description, collapse_ws, comments_before, first_line};
use magic_core::highlight::{Span, token_type};
use magic_core::locals::Locals;
use magic_core::symbols::{SymbolInfo, SymbolKind};
use tree_sitter::{Node, Tree};

pub const BUILTINS: &[&str] = &[
    "print",
    "debugPrint",
    "fatalError",
    "precondition",
    "preconditionFailure",
    "assert",
    "assertionFailure",
    "min",
    "max",
    "abs",
    "zip",
    "stride",
    "type",
    "String",
    "Int",
    "Double",
    "Float",
    "Bool",
    "Character",
    "Array",
    "Dictionary",
    "Set",
    "Optional",
    "Result",
    "Void",
    "Never",
    "Any",
    "AnyObject",
    "Error",
    "Self",
    "self",
    "super",
    "nil",
    "true",
    "false",
];

const COMMENTS: &[&str] = &["comment", "multiline_comment"];

const TYPE_BODIES: &[&str] = &["class_body", "protocol_body", "enum_class_body"];

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    &src[node.start_byte()..node.end_byte()]
}

fn in_type(node: Node) -> bool {
    node.parent().is_some_and(|p| TYPE_BODIES.contains(&p.kind()))
}

fn header(decl: Node, src: &str, body: &str) -> String {
    let end = decl.child_by_field_name(body).map_or(decl.end_byte(), |b| b.start_byte());
    collapse_ws(&src[decl.start_byte()..end])
}

fn docs(decl: Node, src: &str) -> Option<String> {
    comments_before(decl, src, COMMENTS)
}

fn parameters(decl: Node, src: &str) -> Vec<String> {
    let mut cursor = decl.walk();
    decl.named_children(&mut cursor).filter(|c| c.kind() == "parameter").map(|p| collapse_ws(text(p, src))).collect()
}

fn bound_name(decl: Node) -> Option<Node> {
    let pattern = decl.child_by_field_name("name")?;
    pattern.child_by_field_name("bound_identifier").or_else(|| (pattern.kind() == "simple_identifier").then_some(pattern))
}

fn is_static(decl: Node, src: &str) -> bool {
    let mut cursor = decl.walk();
    decl.named_children(&mut cursor).any(|c| c.kind() == "modifiers" && text(c, src).contains("static"))
}

fn type_parameters(class: Node, src: &str) -> Vec<String> {
    let Some(body) = class.child_by_field_name("body") else {
        return Vec::new();
    };
    let mut cursor          = body.walk();
    let children: Vec<Node> = body.named_children(&mut cursor).collect();
    if let Some(init) = children.iter().find(|c| c.kind() == "init_declaration") {
        return parameters(*init, src);
    }
    let is_struct = class.child_by_field_name("declaration_kind").is_some_and(|k| text(k, src) == "struct");
    if !is_struct {
        return Vec::new();
    }
    children
        .iter()
        .filter(|c| c.kind() == "property_declaration" && c.child_by_field_name("computed_value").is_none() && !is_static(**c, src))
        .filter_map(|p| {
            let name       = bound_name(*p)?;
            let mut cursor = p.walk();
            let ty         = p.named_children(&mut cursor).find(|c| c.kind() == "type_annotation");
            Some(match ty {
                Some(t) => collapse_ws(&format!("{}{}", text(name, src), text(t, src))),
                None => text(name, src).to_string(),
            })
        })
        .collect()
}

pub fn symbol(node: Node, src: &str) -> Option<SymbolInfo> {
    let info = |name: Node, kind, detail| SymbolInfo {
        name: text(name, src).to_string(),
        kind,
        selection: (name.start_byte(), name.end_byte()),
        detail,
    };
    match node.kind() {
        "class_declaration" => {
            let name = node.child_by_field_name("name")?;
            let kind = match node.child_by_field_name("declaration_kind").map(|k| text(k, src)) {
                Some("struct") => SymbolKind::Struct,
                Some("enum") => SymbolKind::Enum,
                Some("extension") => SymbolKind::Namespace,
                _ => SymbolKind::Class,
            };
            Some(info(name, kind, None))
        }
        "protocol_declaration" => Some(info(node.child_by_field_name("name")?, SymbolKind::Interface, None)),
        "function_declaration" | "protocol_function_declaration" => {
            let name   = node.child_by_field_name("name")?;
            let kind   = if in_type(node) { SymbolKind::Method } else { SymbolKind::Function };
            let detail = format!("({})", parameters(node, src).join(", "));
            Some(info(name, kind, Some(detail)))
        }
        "init_declaration" => {
            Some(info(node.child_by_field_name("name")?, SymbolKind::Constructor, Some(format!("({})", parameters(node, src).join(", ")))))
        }
        "subscript_declaration" => {
            let mut cursor = node.walk();
            let keyword    = node.children(&mut cursor).find(|c| c.kind() == "subscript")?;
            Some(info(keyword, SymbolKind::Method, None))
        }
        "property_declaration" | "protocol_property_declaration" => {
            let top = node.parent().is_some_and(|p| p.kind() == "source_file");
            if !top && !in_type(node) {
                return None;
            }
            let name       = bound_name(node)?;
            let mut cursor = node.walk();
            let detail = node
                .named_children(&mut cursor)
                .find(|c| c.kind() == "type_annotation")
                .map(|t| collapse_ws(text(t, src).trim_start_matches(':')));
            let kind = if in_type(node) {
                SymbolKind::Property
            } else {
                SymbolKind::Variable
            };
            Some(info(name, kind, detail))
        }
        "enum_entry" => Some(info(node.child_by_field_name("name")?, SymbolKind::EnumMember, None)),
        "typealias_declaration" => Some(info(node.child_by_field_name("name")?, SymbolKind::TypeParameter, None)),
        _ => None,
    }
}

pub fn describe(node: Node, src: &str) -> Option<Description> {
    let parent = node.parent()?;
    let decl   = if parent.kind() == "user_type" { parent.parent()? } else { parent };
    let simple = |signature: String, docs: Option<String>| Description {
        signature,
        docs,
        params: Vec::new(),
    };
    let named = decl.child_by_field_name("name").is_some_and(|n| n == node || n == parent);
    match decl.kind() {
        "function_declaration" | "protocol_function_declaration" if named => Some(Description {
            signature: header(decl, src, "body"),
            docs:      docs(decl, src),
            params:    parameters(decl, src),
        }),
        "class_declaration" | "protocol_declaration" if named => Some(Description {
            signature: header(decl, src, "body"),
            docs:      docs(decl, src),
            params:    type_parameters(decl, src),
        }),
        "typealias_declaration" | "enum_entry" if named => Some(simple(collapse_ws(text(decl, src)), docs(decl, src))),
        "parameter" | "lambda_parameter" if named => Some(simple(format!("(parameter) {}", collapse_ws(text(decl, src))), None)),
        "pattern" if decl.child_by_field_name("bound_identifier") == Some(node) => {
            let owner = decl.parent()?;
            match owner.kind() {
                "property_declaration" | "protocol_property_declaration" => {
                    let end = owner.child_by_field_name("computed_value").map_or(owner.end_byte(), |c| c.start_byte());
                    let sig = first_line(src[owner.start_byte()..end].trim_end(), 120);
                    Some(simple(sig, docs(owner, src)))
                }
                _ => Some(simple(format!("(variable) {}", text(node, src)), None)),
            }
        }
        "if_statement" | "guard_statement" | "while_statement" if decl.child_by_field_name("bound_identifier") == Some(node) => {
            Some(simple(format!("(variable) {}", text(node, src)), None))
        }
        _ => None,
    }
}

pub fn call_at<'t>(node: Node<'t>, offset: usize) -> Option<Call<'t>> {
    let args = std::iter::successors(Some(node), |n| n.parent()).find(|n| {
        n.kind() == "value_arguments"
            && n.parent().is_some_and(|p| p.kind() == "call_suffix")
            && n.start_byte() < offset
            && (offset < n.end_byte() || n.child(n.child_count().wrapping_sub(1) as _).is_none_or(|c| c.kind() != ")" || c.is_missing()))
    })?;
    let call   = args.parent()?.parent().filter(|c| c.kind() == "call_expression")?;
    let target = call.child(0)?;
    let callee = match target.kind() {
        "simple_identifier" => target,
        "navigation_expression" => target.child_by_field_name("suffix")?.child_by_field_name("suffix")?,
        _ => return None,
    };
    Some(Call { callee, args })
}

struct Declared<'s> {
    modules:  Vec<&'s str>,
    cases:    Vec<&'s str>,
    generics: Vec<(&'s str, usize, usize)>,
}

fn declared<'s>(tree: &Tree, src: &'s str) -> Declared<'s> {
    let mut out = Declared {
        modules:  Vec::new(),
        cases:    Vec::new(),
        generics: Vec::new(),
    };
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        match node.kind() {
            "import_declaration" => {
                let module = node.named_child(0).and_then(|i| i.named_child(0));
                out.modules.extend(module.map(|m| text(m, src)));
            }
            "enum_entry" => {
                let mut c = node.walk();
                out.cases.extend(node.children_by_field_name("name", &mut c).map(|n| text(n, src)));
            }
            "type_parameter" => {
                let scope = node.parent().and_then(|p| p.parent());
                if let (Some(name), Some(scope)) = (node.named_child(0), scope) {
                    out.generics.push((text(name, src), scope.start_byte(), scope.end_byte()));
                }
            }
            _ => {}
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    out
}

pub fn refine(tree: &Tree, src: &str, spans: &mut [Span], locals: &Locals) {
    let names = [
        "variable",
        "function",
        "method",
        "property",
        "type",
        "namespace",
        "enumMember",
        "typeParameter",
    ];
    let [variable, function, method, property, ty, namespace, member, generic] = names.map(token_type);

    let found                                                = declared(tree, src);
    let mut cursor                                           = SpanCursor::new(tree);
    let mut cache: HashMap<usize, Vec<(String, SymbolKind)>> = HashMap::new();
    for span in spans.iter_mut() {
        let node = cursor.seek(span.start, span.end);
        if !(node.start_byte() == span.start && node.end_byte() == span.end && node.child_count() == 0) {
            continue;
        }
        let name   = &src[span.start..span.end];
        let parent = cursor.parent().map_or("", |p| p.kind());
        let kind   = span.kind.ty;
        if found.modules.contains(&name) && (kind == ty || kind == variable) {
            span.kind.ty = namespace;
        } else if node.kind() == "type_identifier" && found.generics.iter().any(|&(g, s, e)| g == name && s <= span.start && span.end <= e)
        {
            span.kind.ty = generic;
        } else if found.cases.contains(&name)
            && matches!(parent, "navigation_suffix" | "prefix_expression" | "pattern" | "enum_entry")
            && [variable, function, method, property].contains(&kind)
        {
            span.kind.ty = member;
        } else if node.kind() == "simple_identifier"
            && (kind == variable || kind == function)
            && parent != "navigation_suffix"
            && locals.resolve(name, span.start).is_none()
            && let Some(&body) = cursor.ancestors().iter().rev().find(|n| TYPE_BODIES.contains(&n.kind()))
        {
            let members = cache.entry(body.id()).or_insert_with(|| body_members(body, src));
            match members.iter().find(|(m, _)| m == name).map(|(_, k)| *k) {
                Some(SymbolKind::Property) => span.kind.ty = property,
                Some(SymbolKind::Method) => span.kind.ty = method,
                _ => {}
            }
        }
    }
}

pub fn self_members(node: Node, src: &str) -> Vec<(String, SymbolKind)> {
    match std::iter::successors(Some(node), |n| n.parent()).find(|n| TYPE_BODIES.contains(&n.kind())) {
        Some(body) => body_members(body, src),
        None => Vec::new(),
    }
}

fn body_members(body: Node, src: &str) -> Vec<(String, SymbolKind)> {
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .filter_map(|c| match c.kind() {
            "function_declaration" => c.child_by_field_name("name").map(|n| (text(n, src).to_string(), SymbolKind::Method)),
            "property_declaration" => bound_name(c).map(|n| (text(n, src).to_string(), SymbolKind::Property)),
            _ => None,
        })
        .collect()
}
