use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use magic_core::completion::word_start;
use magic_core::locals::Locals;
use magic_core::symbols::SymbolKind;
use tree_sitter::{Node, Parser, Tree};

use crate::PYTHON;

const MAX_DEPTH: usize = 8;

static SEARCH_PATHS: OnceLock<Vec<PathBuf>> = OnceLock::new();

enum Binding {
    Def(usize, usize),
    From(String, String),
    Module(String),
    Wildcard(String),
}

fn parse(src: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_python::LANGUAGE.into()).ok()?;
    parser.parse(src, None)
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    &src[node.start_byte()..node.end_byte()]
}

fn interpreters(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        for venv in [".venv", "venv", "env"] {
            for bin in ["bin/python", "Scripts/python.exe"] {
                let p = root.join(venv).join(bin);
                if p.is_file() {
                    out.push(p);
                }
            }
        }
    }
    out.extend(["python3", "python"].map(PathBuf::from));
    out
}

fn search_paths(roots: &[PathBuf]) -> &'static [PathBuf] {
    SEARCH_PATHS.get_or_init(|| {
        for python in interpreters(roots) {
            let Ok(res) = Command::new(&python).args(["-c", "import sys\nfor p in sys.path: print(p)"]).output() else {
                continue;
            };
            if res.status.success() {
                return String::from_utf8_lossy(&res.stdout)
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(PathBuf::from)
                    .filter(|p| p.is_dir())
                    .collect();
            }
        }
        Vec::new()
    })
}

fn module_file(base: &Path, segments: &[&str]) -> Option<PathBuf> {
    let mut path = base.to_path_buf();
    for s in segments {
        path.push(s);
    }
    let candidates = match segments.last() {
        Some(last) => vec![
            path.with_file_name(format!("{last}.py")),
            path.join("__init__.py"),
            path.with_file_name(format!("{last}.pyi")),
            path.join("__init__.pyi"),
        ],
        None => vec![path.join("__init__.py"), path.join("__init__.pyi")],
    };
    candidates.into_iter().find(|p| p.is_file())
}

fn bases(file: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for root in roots {
        out.push(root.clone());
        out.push(root.join("src"));
    }
    if let Some(dir) = file.parent() {
        out.push(dir.to_path_buf());
        let mut base = dir;
        while base.join("__init__.py").is_file()
            && let Some(parent) = base.parent()
        {
            base = parent;
        }
        out.push(base.to_path_buf());
    }
    out.extend(search_paths(roots).iter().cloned());
    out
}

fn resolve_module(module: &str, file: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let dots                = module.chars().take_while(|c| *c == '.').count();
    let segments: Vec<&str> = module[dots..].split('.').filter(|s| !s.is_empty()).collect();
    if dots > 0 {
        let mut base = file.parent()?;
        for _ in 1..dots {
            base = base.parent()?;
        }
        return module_file(base, &segments);
    }
    bases(file, roots).iter().find_map(|b| module_file(b, &segments))
}

fn targets(node: Node, out: &mut Vec<(usize, usize)>, src: &str, name: &str) {
    match node.kind() {
        "identifier" if text(node, src) == name => out.push((node.start_byte(), node.end_byte())),
        "pattern_list" | "tuple_pattern" | "list_pattern" | "list_splat_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                targets(child, out, src, name);
            }
        }
        _ => {}
    }
}

fn import_bindings(stmt: Node, src: &str, name: &str, out: &mut Vec<Binding>) {
    let mut cursor = stmt.walk();
    match stmt.kind() {
        "import_statement" => {
            for item in stmt.children_by_field_name("name", &mut cursor) {
                match item.kind() {
                    "dotted_name" => {
                        if item.named_child(0).is_some_and(|first| text(first, src) == name) {
                            out.push(Binding::Module(name.to_string()));
                        }
                    }
                    "aliased_import" => {
                        if let (Some(module), Some(alias)) = (item.child_by_field_name("name"), item.child_by_field_name("alias"))
                            && text(alias, src) == name
                        {
                            out.push(Binding::Module(text(module, src).to_string()));
                        }
                    }
                    _ => {}
                }
            }
        }
        "import_from_statement" => {
            let Some(module) = stmt.child_by_field_name("module_name").map(|m| text(m, src).to_string()) else {
                return;
            };
            if stmt.named_children(&mut cursor).any(|c| c.kind() == "wildcard_import") {
                out.push(Binding::Wildcard(module.clone()));
            }
            for item in stmt.children_by_field_name("name", &mut cursor) {
                let (orig, alias) = match item.kind() {
                    "aliased_import" => (item.child_by_field_name("name"), item.child_by_field_name("alias")),
                    _ => (Some(item), Some(item)),
                };
                if let (Some(orig), Some(alias)) = (orig, alias)
                    && text(alias, src) == name
                {
                    out.push(Binding::From(module.clone(), text(orig, src).to_string()));
                }
            }
        }
        _ => {}
    }
}

fn top_level(node: Node, src: &str, name: &str, out: &mut Vec<Binding>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" | "class_definition" => {
                if let Some(n) = child.child_by_field_name("name").filter(|n| text(*n, src) == name) {
                    out.push(Binding::Def(n.start_byte(), n.end_byte()));
                }
            }
            "expression_statement" => {
                let mut c = child.walk();
                for assignment in child.named_children(&mut c).filter(|a| a.kind() == "assignment") {
                    let mut hits = Vec::new();
                    let mut cur  = Some(assignment);
                    while let Some(a) = cur.filter(|a| a.kind() == "assignment") {
                        if let Some(left) = a.child_by_field_name("left") {
                            targets(left, &mut hits, src, name);
                        }
                        cur = a.child_by_field_name("right");
                    }
                    out.extend(hits.into_iter().map(|(s, e)| Binding::Def(s, e)));
                }
            }
            "import_statement" | "import_from_statement" => import_bindings(child, src, name, out),
            "decorated_definition"
            | "if_statement"
            | "elif_clause"
            | "else_clause"
            | "try_statement"
            | "except_clause"
            | "finally_clause"
            | "with_statement"
            | "block" => top_level(child, src, name, out),
            _ => {}
        }
    }
}

fn identifiers<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    match node.kind() {
        "identifier" => out.push(node),
        "pattern_list" | "tuple_pattern" | "list_pattern" | "list_splat_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                identifiers(child, out);
            }
        }
        _ => {}
    }
}

fn names(node: Node, src: &str, out: &mut Vec<(String, SymbolKind)>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let name = |n: Node| text(n, src).to_string();
        match child.kind() {
            "function_definition" => out.extend(child.child_by_field_name("name").map(|n| (name(n), SymbolKind::Function))),
            "class_definition" => out.extend(child.child_by_field_name("name").map(|n| (name(n), SymbolKind::Class))),
            "expression_statement" => {
                let mut c = child.walk();
                for assignment in child.named_children(&mut c).filter(|a| a.kind() == "assignment") {
                    let mut found = Vec::new();
                    if let Some(left) = assignment.child_by_field_name("left") {
                        identifiers(left, &mut found);
                    }
                    out.extend(found.into_iter().map(|n| (name(n), SymbolKind::Variable)));
                }
            }
            "import_statement" | "import_from_statement" => {
                let mut c = child.walk();
                for item in child.children_by_field_name("name", &mut c) {
                    let bound = match item.kind() {
                        "aliased_import" => item.child_by_field_name("alias"),
                        _ => item.named_child(0),
                    };
                    out.extend(bound.map(|n| (name(n), SymbolKind::Module)));
                }
            }
            "decorated_definition"
            | "if_statement"
            | "elif_clause"
            | "else_clause"
            | "try_statement"
            | "except_clause"
            | "finally_clause"
            | "with_statement"
            | "block" => names(child, src, out),
            _ => {}
        }
    }
}

fn module_names(file: &Path) -> Vec<(String, SymbolKind)> {
    let Some(src) = fs::read(file).ok().map(|b| String::from_utf8_lossy(&b).into_owned()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(tree) = parse(&src) {
        names(tree.root_node(), &src, &mut out);
    }
    if file.file_stem().is_some_and(|s| s == "__init__")
        && let Some(entries) = file.parent().and_then(|d| fs::read_dir(d).ok())
    {
        for entry in entries.flatten() {
            let path   = entry.path();
            let name   = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            let module = if path.is_dir() {
                path.join("__init__.py").is_file()
            } else {
                path.extension().is_some_and(|e| e == "py" || e == "pyi") && name != "__init__"
            };
            if module && !name.is_empty() {
                out.push((name, SymbolKind::Module));
            }
        }
    }
    out
}

pub fn members(tree: &Tree, src: &str, offset: usize, file: &Path, roots: &[PathBuf]) -> Option<Vec<(String, SymbolKind)>> {
    let dot             = word_start(src, offset).checked_sub(1).filter(|&d| src.as_bytes()[d] == b'.')?;
    let start           = src[..dot].rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).map_or(0, |i| i + 1);
    let path: Vec<&str> = src[start..dot].split('.').collect();
    if path.iter().any(|s| s.is_empty()) {
        return None;
    }
    if path == ["self"] {
        let node = tree.root_node().descendant_for_byte_range(dot, dot)?;
        return Some(crate::features::self_members(node, src));
    }
    let mut bindings = Vec::new();
    all_imports(tree.root_node(), src, path[0], &mut bindings);
    let rest: String = path[1..].iter().map(|s| format!(".{s}")).collect();
    bindings.iter().find_map(|binding| {
        let module = match binding {
            Binding::Module(m) => format!("{m}{rest}"),
            Binding::From(m, orig) => format!("{}{rest}", join(m, orig)),
            _ => return None,
        };
        resolve_module(&module, file, roots).map(|target| module_names(&target))
    })
}

fn all_imports(node: Node, src: &str, name: &str, out: &mut Vec<Binding>) {
    if matches!(node.kind(), "import_statement" | "import_from_statement") {
        import_bindings(node, src, name, out);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        all_imports(child, src, name, out);
    }
}

fn resolve_symbol(file: &Path, name: &str, roots: &[PathBuf], depth: usize) -> Vec<(PathBuf, usize, usize)> {
    let Some(src) = fs::read(file).ok().map(|b| String::from_utf8_lossy(&b).into_owned()) else {
        return Vec::new();
    };
    let Some(tree) = parse(&src) else { return Vec::new() };
    let mut found  = Vec::new();
    top_level(tree.root_node(), &src, name, &mut found);

    let defs: Vec<(PathBuf, usize, usize)> = found
        .iter()
        .filter_map(|b| match b {
            Binding::Def(s, e) => Some((file.to_path_buf(), *s, *e)),
            _ => None,
        })
        .collect();
    if !defs.is_empty() || depth >= MAX_DEPTH {
        return defs;
    }
    for binding in &found {
        let hit = match binding {
            Binding::From(module, orig) => resolve_from(module, orig, file, roots, depth + 1),
            Binding::Module(module) => resolve_module(module, file, roots).map(|p| vec![(p, 0, 0)]).unwrap_or_default(),
            _ => Vec::new(),
        };
        if !hit.is_empty() {
            return hit;
        }
    }
    let package = file.file_stem().is_some_and(|s| s == "__init__");
    if package && let Some(sub) = file.parent().and_then(|dir| module_file(dir, &[name])) {
        return vec![(sub, 0, 0)];
    }
    for binding in &found {
        if let Binding::Wildcard(module) = binding
            && let Some(target) = resolve_module(module, file, roots)
        {
            let hit = resolve_symbol(&target, name, roots, depth + 1);
            if !hit.is_empty() {
                return hit;
            }
        }
    }
    Vec::new()
}

fn join(module: &str, name: &str) -> String {
    if module.ends_with('.') { format!("{module}{name}") } else { format!("{module}.{name}") }
}

fn resolve_from(module: &str, name: &str, file: &Path, roots: &[PathBuf], depth: usize) -> Vec<(PathBuf, usize, usize)> {
    let Some(target) = resolve_module(module, file, roots) else {
        return resolve_module(&join(module, name), file, roots).map(|p| vec![(p, 0, 0)]).unwrap_or_default();
    };
    let hit = resolve_symbol(&target, name, roots, depth);
    if hit.is_empty() { vec![(target, 0, 0)] } else { hit }
}

fn resolve_binding(binding: &Binding, file: &Path, roots: &[PathBuf]) -> Vec<(PathBuf, usize, usize)> {
    match binding {
        Binding::From(module, orig) => resolve_from(module, orig, file, roots, 0),
        Binding::Module(module) => resolve_module(module, file, roots).map(|p| vec![(p, 0, 0)]).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn dotted_path(node: Node, src: &str) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" => Some(vec![text(node, src).to_string()]),
        "attribute" => {
            let mut path = dotted_path(node.child_by_field_name("object")?, src)?;
            path.push(text(node.child_by_field_name("attribute")?, src).to_string());
            Some(path)
        }
        _ => None,
    }
}

fn prefix(dotted: Node, upto: Node, src: &str) -> String {
    let mut cursor = dotted.walk();
    dotted
        .named_children(&mut cursor)
        .take_while(|c| c.start_byte() <= upto.start_byte())
        .map(|c| text(c, src))
        .collect::<Vec<_>>()
        .join(".")
}

fn module_of(stmt: Node, src: &str) -> Option<String> {
    stmt.child_by_field_name("module_name").map(|m| text(m, src).to_string())
}

fn import_site(node: Node, src: &str, file: &Path, roots: &[PathBuf]) -> Option<Vec<(PathBuf, usize, usize)>> {
    let parent = node.parent()?;
    let module = |m: String| Some(resolve_module(&m, file, roots).map(|p| vec![(p, 0, 0)]).unwrap_or_default());
    match parent.kind() {
        "dotted_name" => {
            let owner = parent.parent()?;
            let stmt  = if owner.kind() == "aliased_import" || owner.kind() == "relative_import" {
                owner.parent()?
            } else {
                owner
            };
            let path = prefix(parent, node, src);
            match (owner.kind(), stmt.kind()) {
                ("relative_import", _) => {
                    let dots = owner.named_child(0).filter(|p| p.kind() == "import_prefix").map_or("", |p| text(p, src));
                    module(format!("{dots}{path}"))
                }
                (_, "import_statement") => module(path),
                (_, "import_from_statement") if stmt.child_by_field_name("module_name") == Some(parent) => module(path),
                (_, "import_from_statement") => Some(resolve_from(&module_of(stmt, src)?, text(parent, src), file, roots, 0)),
                _ => None,
            }
        }
        "aliased_import" if parent.child_by_field_name("alias") == Some(node) => {
            let orig = parent.child_by_field_name("name")?;
            let stmt = parent.parent()?;
            match stmt.kind() {
                "import_statement" => module(text(orig, src).to_string()),
                "import_from_statement" => Some(resolve_from(&module_of(stmt, src)?, text(orig, src), file, roots, 0)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn attribute_sites(root: Node, src: &str, name: &str) -> Vec<(usize, usize)> {
    let mut init   = Vec::new();
    let mut other  = Vec::new();
    let mut cursor = root.walk();
    'walk: loop {
        let node = cursor.node();
        if node.kind() == "assignment"
            && let Some(left) = node.child_by_field_name("left")
        {
            let target = match left.kind() {
                "attribute" if left.child_by_field_name("object").is_some_and(|o| matches!(text(o, src), "self" | "cls")) => {
                    left.child_by_field_name("attribute")
                }
                "identifier"
                    if node.parent().and_then(|s| s.parent()).and_then(|b| b.parent()).is_some_and(|c| c.kind() == "class_definition") =>
                {
                    Some(left)
                }
                _ => None,
            };
            if let Some(t) = target.filter(|t| text(*t, src) == name) {
                let in_init = std::iter::successors(Some(node), |n| n.parent())
                    .find(|n| n.kind() == "function_definition")
                    .and_then(|f| f.child_by_field_name("name"))
                    .is_some_and(|n| text(n, src) == "__init__");
                let bucket = if in_init || left.kind() == "identifier" {
                    &mut init
                } else {
                    &mut other
                };
                bucket.push((t.start_byte(), t.end_byte()));
            }
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
    if init.is_empty() { other } else { init }
}

fn usage(tree: &Tree, node: Node, src: &str, file: &Path, roots: &[PathBuf]) -> Vec<(PathBuf, usize, usize)> {
    let root   = tree.root_node();
    let name   = text(node, src);
    let parent = node.parent();

    if let Some(attr) = parent.filter(|p| p.kind() == "attribute" && p.child_by_field_name("attribute") == Some(node))
        && let Some(path) = attr.child_by_field_name("object").and_then(|o| dotted_path(o, src))
    {
        let mut bindings = Vec::new();
        all_imports(root, src, &path[0], &mut bindings);
        let rest: String = path[1..].iter().map(|s| format!(".{s}")).collect();
        for binding in &bindings {
            let module = match binding {
                Binding::Module(m) => format!("{m}{rest}"),
                Binding::From(m, orig) => format!("{}{rest}", join(m, orig)),
                _ => continue,
            };
            if let Some(target) = resolve_module(&module, file, roots) {
                return resolve_symbol(&target, name, roots, 0);
            }
        }
        if !bindings.is_empty() {
            return Vec::new();
        }
        return attribute_sites(root, src, name).into_iter().map(|(s, e)| (file.to_path_buf(), s, e)).collect();
    }

    let locals = Locals::collect(&PYTHON, tree, src);
    if let Some(def) = locals.resolve(name, node.start_byte()) {
        let site = root.descendant_for_byte_range(def.start, def.end);
        let imported = std::iter::successors(site, |n| n.parent())
            .any(|n| matches!(n.kind(), "import_statement" | "import_from_statement"));
        if !imported {
            return Vec::new();
        }
    }
    let mut own = Vec::new();
    top_level(root, src, name, &mut own);
    if own.iter().any(|b| matches!(b, Binding::Def(..))) {
        return Vec::new();
    }
    let mut bindings = Vec::new();
    all_imports(root, src, name, &mut bindings);
    bindings.iter().map(|b| resolve_binding(b, file, roots)).find(|hit| !hit.is_empty()).unwrap_or_default()
}

pub fn definitions(tree: &Tree, src: &str, offset: usize, file: &Path, roots: &[PathBuf]) -> Vec<(PathBuf, usize, usize)> {
    let root = tree.root_node();
    let Some(node) = [offset, offset.saturating_sub(1)]
        .into_iter()
        .filter_map(|o| root.descendant_for_byte_range(o, o))
        .find(|n| n.kind() == "identifier")
    else {
        return Vec::new();
    };
    import_site(node, src, file, roots).unwrap_or_else(|| usage(tree, node, src, file, roots))
}
