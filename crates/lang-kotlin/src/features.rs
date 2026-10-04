use std::collections::{HashMap, HashSet};

use magic_core::describe::{Call, Description, collapse_ws, comments_before, first_line};
use magic_core::highlight::{Span, token_type};
use magic_core::locals::Locals;
use magic_core::symbols::{SymbolInfo, SymbolKind};
use tree_sitter::{Node, Tree};

use crate::KOTLIN;

pub const BUILTINS: &[&str] = &[
    "println",
    "print",
    "readln",
    "readLine",
    "listOf",
    "listOfNotNull",
    "mutableListOf",
    "arrayListOf",
    "emptyList",
    "mapOf",
    "mutableMapOf",
    "hashMapOf",
    "emptyMap",
    "setOf",
    "mutableSetOf",
    "hashSetOf",
    "emptySet",
    "arrayOf",
    "intArrayOf",
    "emptyArray",
    "sequenceOf",
    "buildList",
    "buildMap",
    "buildString",
    "lazy",
    "require",
    "requireNotNull",
    "check",
    "checkNotNull",
    "error",
    "TODO",
    "repeat",
    "run",
    "let",
    "also",
    "apply",
    "with",
    "takeIf",
    "takeUnless",
    "maxOf",
    "minOf",
    "to",
    "String",
    "Int",
    "Long",
    "Short",
    "Byte",
    "Double",
    "Float",
    "Boolean",
    "Char",
    "Unit",
    "Any",
    "Nothing",
    "Array",
    "List",
    "MutableList",
    "Map",
    "MutableMap",
    "Set",
    "MutableSet",
    "Pair",
    "Triple",
    "Sequence",
    "Exception",
    "it",
    "this",
    "super",
    "null",
    "true",
    "false",
    "break",
    "continue",
];

const OPERATOR_FUNCTIONS: &[&str] = &[
    "getValue",
    "setValue",
    "provideDelegate",
    "invoke",
    "get",
    "set",
    "contains",
    "iterator",
    "next",
    "hasNext",
    "compareTo",
    "equals",
    "plus",
    "minus",
    "times",
    "div",
    "rem",
    "rangeTo",
    "rangeUntil",
    "unaryPlus",
    "unaryMinus",
    "not",
    "inc",
    "dec",
    "plusAssign",
    "minusAssign",
    "timesAssign",
    "divAssign",
    "remAssign",
];

const COMMENTS: &[&str] = &["line_comment", "block_comment"];

const TYPE_BODIES: &[&str] = &["class_body", "enum_class_body"];

const BODIES: &[&str] = &["function_body", "class_body", "enum_class_body", "getter", "setter"];

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    &src[node.start_byte()..node.end_byte()]
}

fn child_of_kind<'t>(node: Node<'t>, kinds: &[&str]) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| kinds.contains(&c.kind()))
}

fn has_token(node: Node, src: &str, token: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|c| match c.kind() {
        "modifiers" => text(c, src).split_whitespace().any(|w| w == token),
        kind => !c.is_named() && kind == token,
    })
}

fn in_type(node: Node) -> bool {
    node.parent().is_some_and(|p| TYPE_BODIES.contains(&p.kind()))
}

fn header(decl: Node, src: &str) -> String {
    let end = child_of_kind(decl, BODIES).map_or(decl.end_byte(), |b| b.start_byte());
    collapse_ws(&src[decl.start_byte()..end])
}

fn docs(decl: Node, src: &str) -> Option<String> {
    comments_before(decl, src, COMMENTS)
}

fn split_list(list: Node, src: &str) -> Vec<String> {
    let mut out                          = Vec::new();
    let mut span: Option<(usize, usize)> = None;
    let mut cursor                       = list.walk();
    for child in list.children(&mut cursor) {
        match child.kind() {
            "(" | ")" | "," => {
                if let Some((start, end)) = span.take() {
                    out.push(collapse_ws(&src[start..end]));
                }
            }
            kind if COMMENTS.contains(&kind) => {}
            _ => span = Some((span.map_or(child.start_byte(), |s| s.0), child.end_byte())),
        }
    }
    out
}

fn parameters(decl: Node, src: &str) -> Vec<String> {
    child_of_kind(decl, &["function_value_parameters"]).map(|p| split_list(p, src)).unwrap_or_default()
}

fn constructor_parameters(class: Node, src: &str) -> Vec<String> {
    child_of_kind(class, &["primary_constructor"])
        .and_then(|c| child_of_kind(c, &["class_parameters"]))
        .map(|p| split_list(p, src))
        .unwrap_or_default()
}

fn property_name(decl: Node) -> Option<Node> {
    let var = child_of_kind(decl, &["variable_declaration"])?;
    child_of_kind(var, &["identifier"])
}

fn is_property_parameter(param: Node, src: &str) -> bool {
    has_token(param, src, "val") || has_token(param, src, "var")
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
            let kind = if has_token(node, src, "interface") {
                SymbolKind::Interface
            } else if has_token(node, src, "enum") {
                SymbolKind::Enum
            } else {
                SymbolKind::Class
            };
            Some(info(name, kind, None))
        }
        "object_declaration" => Some(info(node.child_by_field_name("name")?, SymbolKind::Class, Some("object".into()))),
        "companion_object" => {
            let name = node.child_by_field_name("name").or_else(|| child_of_kind(node, &["companion"]))?;
            Some(info(name, SymbolKind::Class, Some("companion object".into())))
        }
        "function_declaration" => {
            let name   = node.child_by_field_name("name")?;
            let kind   = if in_type(node) { SymbolKind::Method } else { SymbolKind::Function };
            let detail = format!("({})", parameters(node, src).join(", "));
            Some(info(name, kind, Some(detail)))
        }
        "secondary_constructor" => {
            let keyword = child_of_kind(node, &["constructor"])?;
            Some(info(keyword, SymbolKind::Constructor, Some(format!("({})", parameters(node, src).join(", ")))))
        }
        "property_declaration" => {
            let top = node.parent().is_some_and(|p| p.kind() == "source_file");
            if !top && !in_type(node) {
                return None;
            }
            let name   = property_name(node)?;
            let detail = name.next_named_sibling().map(|t| collapse_ws(text(t, src)));
            let kind   = if in_type(node) {
                SymbolKind::Property
            } else {
                SymbolKind::Variable
            };
            Some(info(name, kind, detail))
        }
        "class_parameter" if is_property_parameter(node, src) => {
            let name   = child_of_kind(node, &["identifier"])?;
            let detail = name.next_named_sibling().filter(|t| t.kind().ends_with("type")).map(|t| collapse_ws(text(t, src)));
            Some(info(name, SymbolKind::Property, detail))
        }
        "enum_entry" => Some(info(child_of_kind(node, &["identifier"])?, SymbolKind::EnumMember, None)),
        "type_alias" => Some(info(node.child_by_field_name("type")?, SymbolKind::TypeParameter, None)),
        _ => None,
    }
}

pub fn describe(node: Node, src: &str) -> Option<Description> {
    let decl   = node.parent()?;
    let simple = |signature: String, docs: Option<String>| Description {
        signature,
        docs,
        params: Vec::new(),
    };
    let implicit = text(node, src) == "it"
        && std::iter::successors(Some(node), |n| n.parent())
            .find(|n| n.kind() == "lambda_literal")
            .is_some_and(|l| child_of_kind(l, &["lambda_parameters"]).is_none());
    if implicit {
        return Some(simple("(parameter) it".into(), None));
    }
    let named = decl.child_by_field_name("name") == Some(node);
    let owner = if decl.kind() == "qualified_identifier" {
        decl.parent()
    } else {
        Some(decl)
    };
    if let Some(import) = owner.filter(|o| o.kind() == "import") {
        return Some(simple(collapse_ws(text(import, src)), None));
    }
    match decl.kind() {
        "function_declaration" if named => Some(Description {
            signature: header(decl, src),
            docs:      docs(decl, src),
            params:    parameters(decl, src),
        }),
        "class_declaration" | "object_declaration" | "companion_object" if named => Some(Description {
            signature: header(decl, src),
            docs:      docs(decl, src),
            params:    constructor_parameters(decl, src),
        }),
        "type_alias" if decl.child_by_field_name("type") == Some(node) => Some(simple(collapse_ws(text(decl, src)), docs(decl, src))),
        "enum_entry" => Some(simple(first_line(text(decl, src), 120), docs(decl, src))),
        "parameter" => Some(simple(format!("(parameter) {}", collapse_ws(text(decl, src))), None)),
        "class_parameter" => {
            let kind = if is_property_parameter(decl, src) {
                "property"
            } else {
                "parameter"
            };
            Some(simple(format!("({kind}) {}", collapse_ws(text(decl, src))), None))
        }
        "catch_block" => {
            let end = child_of_kind(decl, &["block"]).map_or(decl.end_byte(), |b| b.start_byte());
            let sig = src[node.start_byte()..end].trim_end().trim_end_matches(')').trim_end();
            Some(simple(format!("(variable) {}", collapse_ws(sig)), None))
        }
        "variable_declaration" => {
            let owner = decl.parent()?;
            match owner.kind() {
                "property_declaration" => {
                    let end = child_of_kind(owner, &["getter", "setter"]).map_or(owner.end_byte(), |c| c.start_byte());
                    let sig = first_line(src[owner.start_byte()..end].trim_end(), 120);
                    Some(simple(sig, docs(owner, src)))
                }
                _ => Some(simple(format!("(variable) {}", collapse_ws(text(decl, src))), None)),
            }
        }
        _ => None,
    }
}

pub fn is_definition(name: Node) -> bool {
    let Some(decl) = name.parent().filter(|p| p.kind() == "variable_declaration") else {
        return true;
    };
    let owner = decl.parent().map(|p| {
        if p.kind() == "multi_variable_declaration" {
            p.parent()
        } else {
            Some(p)
        }
    });
    match owner.flatten() {
        Some(property) if property.kind() == "property_declaration" => {
            property.parent().is_some_and(|p| matches!(p.kind(), "source_file" | "class_body" | "enum_class_body"))
        }
        _ => false,
    }
}

fn supertype_name(specifier: Node) -> Option<Node> {
    let mut node = specifier;
    loop {
        node = match node.kind() {
            "user_type" => return node.named_children(&mut node.walk()).filter(|c| c.kind() == "identifier").last(),
            "delegation_specifier" | "constructor_invocation" | "explicit_delegation" | "nullable_type" => {
                node.named_children(&mut node.walk()).find(|c| c.kind() != "annotation")?
            }
            _ => return None,
        };
    }
}

pub fn implementations(tree: &Tree, src: &str, name: &str) -> Vec<(usize, usize)> {
    let mut out    = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        match node.kind() {
            "class_declaration" | "object_declaration" | "companion_object" | "object_literal" => {
                let supertypes = child_of_kind(node, &["delegation_specifiers"]);
                let extends    = supertypes.is_some_and(|s| {
                    let mut c = s.walk();
                    s.named_children(&mut c).filter_map(supertype_name).any(|n| text(n, src) == name)
                });
                let target = node.child_by_field_name("name").or_else(|| child_of_kind(node, &["object", "companion"]));
                if let Some(target) = target.filter(|_| extends) {
                    out.push((target.start_byte(), target.end_byte()));
                }
            }
            "function_declaration" | "property_declaration" if has_token(node, src, "override") => {
                let target = node.child_by_field_name("name").or_else(|| property_name(node));
                if let Some(target) = target.filter(|t| text(*t, src) == name) {
                    out.push((target.start_byte(), target.end_byte()));
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

pub fn import_of(tree: &Tree, src: &str, name: &str) -> Option<(usize, usize)> {
    let root       = tree.root_node();
    let mut cursor = root.walk();
    root.named_children(&mut cursor).filter(|c| c.kind() == "import").find_map(|import| {
        let target = match child_of_kind(import, &["identifier"]) {
            Some(alias) => alias,
            None => {
                let path = child_of_kind(import, &["qualified_identifier"])?;
                if child_of_kind(import, &["*"]).is_some() {
                    return None;
                }
                path.named_child(path.named_child_count().checked_sub(1)? as _)?
            }
        };
        (text(target, src) == name).then(|| (target.start_byte(), target.end_byte()))
    })
}

pub fn call_at<'t>(node: Node<'t>, offset: usize) -> Option<Call<'t>> {
    let args = std::iter::successors(Some(node), |n| n.parent()).find(|n| {
        n.kind() == "value_arguments"
            && n.parent().is_some_and(|p| p.kind() == "call_expression")
            && n.start_byte() < offset
            && (offset < n.end_byte() || n.child(n.child_count().wrapping_sub(1) as _).is_none_or(|c| c.kind() != ")" || c.is_missing()))
    })?;
    let target = args.parent()?.child(0)?;
    let callee = match target.kind() {
        "identifier" => target,
        "navigation_expression" => {
            target.named_child(target.named_child_count().checked_sub(1)? as _).filter(|n| n.kind() == "identifier")?
        }
        _ => return None,
    };
    Some(Call { callee, args })
}

struct Declared<'s> {
    cases:    Vec<&'s str>,
    generics: Vec<(&'s str, usize, usize)>,
    called:   HashSet<&'s str>,
}

fn callee(call: Node) -> Option<Node> {
    let target = call.child(0)?;
    match target.kind() {
        "identifier" => Some(target),
        "navigation_expression" => target.named_child(target.named_child_count().checked_sub(1)? as _).filter(|n| n.kind() == "identifier"),
        _ => None,
    }
}

fn declared<'s>(tree: &Tree, src: &'s str) -> Declared<'s> {
    let mut out = Declared {
        cases:    Vec::new(),
        generics: Vec::new(),
        called:   HashSet::new(),
    };
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        match node.kind() {
            "enum_entry" => out.cases.extend(child_of_kind(node, &["identifier"]).map(|n| text(n, src))),
            "call_expression" => out.called.extend(callee(node).map(|n| text(n, src))),
            "callable_reference" => {
                let mut c = node.walk();
                out.called.extend(node.named_children(&mut c).filter(|n| n.kind() == "identifier").last().map(|n| text(n, src)));
            }
            "type_parameter" => {
                let scope = node.parent().and_then(|p| p.parent());
                if let (Some(name), Some(scope)) = (child_of_kind(node, &["identifier"]), scope) {
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

pub fn refine(tree: &Tree, src: &str, spans: &mut [Span]) {
    let names = [
        "variable",
        "function",
        "method",
        "property",
        "type",
        "enumMember",
        "typeParameter",
    ];
    let [variable, function, method, property, ty, member, generic] = names.map(token_type);

    let found                                                = declared(tree, src);
    let locals                                               = Locals::collect(&KOTLIN, tree, src);
    let root                                                 = tree.root_node();
    let mut cache: HashMap<usize, Vec<(String, SymbolKind)>> = HashMap::new();
    for span in spans.iter_mut() {
        let node       = root.descendant_for_byte_range(span.start, span.end);
        let Some(node) = node.filter(|n| n.start_byte() == span.start && n.end_byte() == span.end && n.kind() == "identifier") else {
            continue;
        };
        let name     = &src[span.start..span.end];
        let parent   = node.parent().map_or("", |p| p.kind());
        let kind     = span.kind.ty;
        let accessed = parent == "navigation_expression" && node.prev_sibling().is_some();
        let imported = parent == "qualified_identifier" && node.parent().and_then(|p| p.parent()).is_some_and(|i| i.kind() == "import");
        if kind == function
            && imported
            && !found.called.contains(name)
            && !OPERATOR_FUNCTIONS.contains(&name)
            && !name.starts_with("component")
        {
            span.kind.ty = property;
        } else if kind == function && parent == "function_declaration" && node.parent().is_some_and(in_type) {
            span.kind.ty = method;
        } else if parent == "class_parameter" && node.parent().is_some_and(|p| is_property_parameter(p, src)) {
            span.kind.ty = property;
        } else if parent == "user_type" && found.generics.iter().any(|&(g, s, e)| g == name && s <= span.start && span.end <= e) {
            span.kind.ty = generic;
        } else if found.cases.contains(&name)
            && matches!(parent, "navigation_expression" | "when_entry" | "enum_entry")
            && [variable, function, method, property, ty].contains(&kind)
        {
            span.kind.ty = member;
        } else if (kind == variable || kind == function)
            && !accessed
            && locals.resolve(name, span.start).is_none()
            && let Some(body) = std::iter::successors(Some(node), |n| n.parent()).find(|n| TYPE_BODIES.contains(&n.kind()))
        {
            let members = cache.entry(body.id()).or_insert_with(|| self_members(node, src));
            match members.iter().find(|(m, _)| m == name).map(|(_, k)| *k) {
                Some(SymbolKind::Property) => span.kind.ty = property,
                Some(SymbolKind::Method) if parent == "call_expression" => span.kind.ty = method,
                _ => {}
            }
        }
    }
}

pub fn self_members(node: Node, src: &str) -> Vec<(String, SymbolKind)> {
    let Some(body) = std::iter::successors(Some(node), |n| n.parent()).find(|n| TYPE_BODIES.contains(&n.kind())) else {
        return Vec::new();
    };
    let mut out: Vec<(String, SymbolKind)> = body
        .parent()
        .and_then(|class| child_of_kind(class, &["primary_constructor"]))
        .and_then(|c| child_of_kind(c, &["class_parameters"]))
        .map(|params| {
            let mut cursor = params.walk();
            params
                .named_children(&mut cursor)
                .filter(|p| is_property_parameter(*p, src))
                .filter_map(|p| child_of_kind(p, &["identifier"]))
                .map(|n| (text(n, src).to_string(), SymbolKind::Property))
                .collect()
        })
        .unwrap_or_default();
    let mut cursor = body.walk();
    out.extend(body.named_children(&mut cursor).filter_map(|c| match c.kind() {
        "function_declaration" => c.child_by_field_name("name").map(|n| (text(n, src).to_string(), SymbolKind::Method)),
        "property_declaration" => property_name(c).map(|n| (text(n, src).to_string(), SymbolKind::Property)),
        _ => None,
    }));
    out
}
