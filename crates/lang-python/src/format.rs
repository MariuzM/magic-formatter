use std::collections::{HashMap, HashSet};

use tree_sitter::{Node, Parser, Tree};

pub struct Options {
    pub indent:    String,
    pub max_width: usize,
}

const ATOMIC: &[&str] = &["string", "comment"];

const BINARY_PARENTS: &[&str] = &[
    "binary_operator",
    "comparison_operator",
    "boolean_operator",
    "assignment",
    "augmented_assignment",
    "named_expression",
    "typed_default_parameter",
    "type_alias_statement",
    "conditional_expression",
];

const SPLATS: &[&str] = &["list_splat", "dictionary_splat", "list_splat_pattern", "dictionary_splat_pattern"];

const HEADERS: &[&str] = &[
    "function_definition",
    "class_definition",
    "if_statement",
    "elif_clause",
    "while_statement",
    "for_statement",
    "with_statement",
    "match_statement",
    "case_clause",
    "except_clause",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Break {
    None,
    Line,
    Blank,
    Blank2,
}

struct Tok<'t> {
    node:      Node<'t>,
    start:     usize,
    end:       usize,
    kind:      &'t str,
    parent:    &'t str,
    named:     bool,
    comment:   bool,
    multiline: bool,
}

struct Line {
    first: usize,
    last:  usize,
    level: usize,
}

fn tokens<'t>(tree: &'t Tree, src: &str) -> Vec<Tok<'t>> {
    let mut out    = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let atom = ATOMIC.contains(&node.kind());
        let raw  = &src[node.start_byte()..node.end_byte()];
        let text = raw.trim();
        if (atom || node.child_count() == 0) && !text.is_empty() {
            let kind  = node.kind();
            let start = node.start_byte() + (raw.len() - raw.trim_start().len());
            out.push(Tok {
                node,
                start,
                end: start + text.len(),
                kind,
                parent: node.parent().map_or("", |p| p.kind()),
                named: node.is_named(),
                comment: kind == "comment",
                multiline: text.contains('\n'),
            });
        }
        if !atom && cursor.goto_first_child() {
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
    fill_gaps(src, out)
}

fn fill_gaps<'t>(src: &str, toks: Vec<Tok<'t>>) -> Vec<Tok<'t>> {
    let mut out = Vec::new();
    let mut pos = 0;
    for tok in &toks {
        let mut k = pos;
        while k < tok.start {
            let c = src[k..].chars().next().unwrap();
            if c.is_whitespace() {
                k += c.len_utf8();
                continue;
            }
            let start = k;
            while k < tok.start && !src[k..].starts_with(char::is_whitespace) {
                k += src[k..].chars().next().unwrap().len_utf8();
            }
            out.push(Tok {
                node: tok.node,
                start,
                end: k,
                kind: if &src[start..k] == "\\" { "line_continuation" } else { "ERROR" },
                parent: "",
                named: false,
                comment: false,
                multiline: false,
            });
        }
        pos = pos.max(tok.end);
    }
    let mut toks = toks;
    if out.is_empty() {
        return toks;
    }
    toks.append(&mut out);
    toks.sort_by_key(|t| t.start);
    toks
}

fn first_error(node: Node) -> Option<Node> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    if !node.has_error() {
        return None;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(first_error)
}

fn parse(src: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_python::LANGUAGE.into()).map_err(|e| e.to_string())?;
    parser.parse(src, None).ok_or_else(|| "parser failed".into())
}

fn is_open(kind: &str) -> bool {
    matches!(kind, "(" | "[" | "{")
}

fn is_close(kind: &str) -> bool {
    matches!(kind, ")" | "]" | "}")
}

fn block_depth(node: Node) -> usize {
    let mut depth = 0;
    let mut node  = node;
    while let Some(parent) = node.parent() {
        if parent.kind() == "block" {
            depth += 1;
        }
        node = parent;
    }
    depth
}

fn comment_depth(node: Node) -> usize {
    let mut next = node.next_named_sibling();
    while let Some(n) = next.filter(|n| n.kind() == "comment") {
        next = n.next_named_sibling();
    }
    block_depth(node) + usize::from(next.is_some_and(|n| n.kind() == "block"))
}

fn in_header(node: Node) -> bool {
    let mut node = node;
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "block" | "module" => return false,
            k if HEADERS.contains(&k) => return true,
            _ => {}
        }
        node = parent;
    }
    false
}

fn is_docstring(node: Node) -> bool {
    let Some(stmt) = node.parent().filter(|p| p.kind() == "expression_statement" && p.named_child_count() == 1) else {
        return false;
    };
    let Some(body) = stmt.parent() else { return false };
    let mut cursor = body.walk();
    matches!(body.kind(), "block" | "module") && body.named_children(&mut cursor).find(|c| c.kind() != "comment") == Some(stmt)
}

struct Formatter<'a, 't> {
    src:     &'a str,
    opts:    &'a Options,
    toks:    Vec<Tok<'t>>,
    brk:     Vec<Break>,
    ws:      Vec<bool>,
    space:   Vec<bool>,
    forced:  Vec<bool>,
    pair:    Vec<usize>,
    depth:   Vec<usize>,
    lines:   Vec<Line>,
    padding: Vec<usize>,
}

impl<'a, 't> Formatter<'a, 't> {
    fn new(src: &'a str, opts: &'a Options, toks: Vec<Tok<'t>>) -> Result<Self, String> {
        let n         = toks.len();
        let mut brk   = vec![Break::None; n];
        let mut ws    = vec![false; n];
        let mut pair  = vec![usize::MAX; n];
        let mut depth = vec![0; n];
        let mut stack = Vec::new();
        for i in 0..n {
            if i > 0 {
                let gap = &src[toks[i - 1].end..toks[i].start];
                ws[i]   = !gap.is_empty();
                brk[i]  = match gap.matches('\n').count() {
                    0 if toks[i - 1].kind == "line_continuation" => Break::Line,
                    0 => Break::None,
                    1 => Break::Line,
                    2 => Break::Blank,
                    _ => Break::Blank2,
                };
            }
            depth[i] = stack.len();
            if is_open(toks[i].kind) {
                stack.push(i);
            } else if is_close(toks[i].kind) {
                let o   = stack.pop().ok_or("unbalanced brackets")?;
                pair[o] = i;
                pair[i] = o;
            }
        }
        if !stack.is_empty() {
            return Err("unbalanced brackets".into());
        }
        let padding = toks.iter().map(|t| usize::from(t.comment)).collect();
        Ok(Self {
            src,
            opts,
            toks,
            brk,
            ws,
            space: vec![false; n],
            forced: vec![false; n],
            pair,
            depth,
            lines: Vec::new(),
            padding,
        })
    }

    fn text(&self, i: usize) -> &'a str {
        &self.src[self.toks[i].start..self.toks[i].end]
    }

    fn break_before(&mut self, mut i: usize) {
        while i < self.toks.len() && self.toks[i].comment && self.brk[i] == Break::None && i > 0 {
            i += 1;
        }
        if i < self.toks.len() {
            if self.brk[i] == Break::None {
                self.brk[i] = Break::Line;
            }
            self.forced[i] = true;
        }
    }

    fn top_level_commas(&self, open: usize) -> Vec<usize> {
        let close      = self.pair[open];
        let mut commas = Vec::new();
        let mut i      = open + 1;
        while i < close {
            if is_open(self.toks[i].kind) {
                i = self.pair[i];
            } else if self.toks[i].kind == "," {
                commas.push(i);
            }
            i += 1;
        }
        commas
    }

    fn last_code_before(&self, i: usize) -> usize {
        let mut j = i - 1;
        while j > 0 && self.toks[j].comment {
            j -= 1;
        }
        j
    }

    fn apply_groups(&mut self) -> Vec<usize> {
        let mut candidates = Vec::new();
        for open in 0..self.toks.len() {
            if !is_open(self.toks[open].kind) {
                continue;
            }
            let close  = self.pair[open];
            let commas = self.top_level_commas(open);
            if commas.is_empty() {
                continue;
            }
            let singleton  = commas.len() == 1 && matches!(self.toks[open].parent, "tuple" | "tuple_pattern");
            let trailing   = self.toks[self.last_code_before(close)].kind == "," && !singleton;
            let multiline  = (open + 1..=close).any(|k| self.brk[k] != Break::None);
            let first_next = self.brk[open + 1] != Break::None;
            if trailing || multiline && first_next {
                self.break_before(open + 1);
                for &c in &commas {
                    if c + 1 != close {
                        self.break_before(c + 1);
                    }
                }
                self.break_before(close);
            } else if multiline {
                candidates.push(open);
            }
        }
        candidates
    }

    fn chain_dots(&self, node: Node, dots: &mut Vec<usize>, seen: &mut HashSet<usize>) {
        match node.kind() {
            "attribute" => {
                seen.insert(node.id());
                let mut cursor = node.walk();
                if let Some(dot) = node.children(&mut cursor).find(|c| c.kind() == ".") {
                    dots.push(dot.start_byte());
                }
                if let Some(object) = node.child_by_field_name("object") {
                    self.chain_dots(object, dots, seen);
                }
            }
            "call" => {
                if let Some(function) = node.child_by_field_name("function") {
                    self.chain_dots(function, dots, seen);
                }
            }
            "subscript" => {
                if let Some(value) = node.child_by_field_name("value") {
                    self.chain_dots(value, dots, seen);
                }
            }
            _ => {}
        }
    }

    fn apply_chains(&mut self, tree: &Tree) {
        let by_start: HashMap<usize, usize> =
            self.toks.iter().enumerate().filter(|(_, t)| t.kind == ".").map(|(i, t)| (t.start, i)).collect();
        let mut seen   = HashSet::new();
        let mut cursor = tree.walk();
        'walk: loop {
            let node = cursor.node();
            if node.kind() == "attribute" && !seen.contains(&node.id()) {
                let mut dots = Vec::new();
                self.chain_dots(node, &mut dots, &mut seen);
                dots.sort_unstable();
                let idx: Vec<usize> = dots.iter().filter_map(|d| by_start.get(d).copied()).collect();
                let breakable = idx.len() == dots.len()
                    && idx.first().is_some_and(|&first| {
                        let last = *idx.last().unwrap();
                        self.brk[first] != Break::None
                            && self.depth[first] > 0
                            && !(first - 1..=last).any(|k| self.toks[k].kind == "line_continuation")
                    });
                if breakable {
                    for i in idx {
                        self.break_before(i);
                    }
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
    }

    fn is_binary(&self, i: usize) -> bool {
        let t = &self.toks[i];
        !t.named
            && !matches!(t.kind, "(" | ")" | "[" | "]" | "{" | "}" | "," | ":")
            && (matches!(t.kind, "->" | ":=") || BINARY_PARENTS.contains(&t.parent))
    }

    fn is_call_paren(&self, i: usize) -> bool {
        let t = &self.toks[i];
        match (t.kind, t.parent) {
            ("(", "argument_list" | "parameters") => true,
            ("(", "generator_expression") => t.node.parent().and_then(|p| p.parent()).is_some_and(|g| g.kind() == "call"),
            ("[", "subscript" | "type_parameter") => true,
            _ => false,
        }
    }

    fn space_before(&self, i: usize) -> bool {
        let a  = &self.toks[i - 1];
        let b  = &self.toks[i];
        let ws = self.ws[i];
        if b.comment {
            return true;
        }
        if a.comment || matches!(b.kind, "line_continuation" | "ERROR") || a.kind == "ERROR" {
            return ws;
        }
        if matches!(b.kind, "," | ";" | ")" | "]" | "}") || is_open(a.kind) {
            return false;
        }
        if matches!(a.kind, "," | ";") {
            return true;
        }
        if b.kind == ":" {
            return b.parent == "slice" && ws;
        }
        if a.kind == ":" {
            return a.parent != "slice" || ws;
        }
        if a.kind == "." || b.kind == "." {
            return (a.parent == "import_prefix" || b.parent == "import_prefix" || a.kind == "integer") && ws;
        }
        if self.is_call_paren(i) {
            return false;
        }
        if matches!(a.kind, "-" | "+" | "~") && a.parent == "unary_operator" {
            return false;
        }
        if matches!(a.kind, "*" | "**") && SPLATS.contains(&a.parent) {
            return false;
        }
        if a.kind == "@" && a.parent == "decorator" {
            return false;
        }
        if (a.kind == "=" && matches!(a.parent, "keyword_argument" | "default_parameter"))
            || (b.kind == "=" && matches!(b.parent, "keyword_argument" | "default_parameter"))
        {
            return false;
        }
        if (a.kind == "**" && a.parent == "binary_operator") || (b.kind == "**" && b.parent == "binary_operator") {
            return ws;
        }
        if self.is_binary(i - 1) || self.is_binary(i) {
            return true;
        }
        ws
    }

    fn compute_lines(&mut self) {
        let n         = self.toks.len();
        let mut lines = Vec::new();
        let mut first = 0;
        for i in 1..=n {
            if i == n || self.brk[i] != Break::None || self.toks[i - 1].comment {
                lines.push(Line {
                    first,
                    last: i - 1,
                    level: 0,
                });
                first = i;
            }
        }
        for i in 1..n {
            if self.toks[i - 1].comment && self.brk[i] == Break::None {
                self.brk[i] = Break::Line;
            }
        }

        let mut stack: Vec<usize> = Vec::new();
        let mut anchor            = vec![0; n];
        let mut deferred          = Vec::new();
        let mut statement         = 0;
        for li in 0..lines.len() {
            let (lf, ll)    = (lines[li].first, lines[li].last);
            let code        = (lf..=ll).find(|&k| !self.toks[k].comment);
            lines[li].level = match (code, stack.last()) {
                (None, Some(_)) => {
                    deferred.push(li);
                    0
                }
                (Some(t), Some(_)) if is_close(self.toks[t].kind) => lines[anchor[self.pair[t]]].level,
                (Some(_), Some(&o)) => {
                    let hanging = self.brk[self.pair[o]] == Break::None && in_header(self.toks[o].node);
                    lines[anchor[o]].level + 1 + usize::from(hanging)
                }
                (Some(_), None) if lf > 0 && self.toks[lf - 1].kind == "line_continuation" => statement + 1,
                (Some(t), None) => {
                    statement = block_depth(self.toks[t].node);
                    statement
                }
                (None, None) => comment_depth(self.toks[lf].node),
            };
            for k in lf..=ll {
                if is_open(self.toks[k].kind) {
                    anchor[k] = li;
                    stack.push(k);
                } else if is_close(self.toks[k].kind) {
                    stack.pop();
                }
            }
        }
        for li in deferred.into_iter().rev() {
            let next        = (li + 1..lines.len()).find(|&l| (lines[l].first..=lines[l].last).any(|k| !self.toks[k].comment));
            lines[li].level = match next {
                Some(l) if is_close(self.toks[lines[l].first].kind) => lines[l].level + 1,
                Some(l) => lines[l].level,
                None => 0,
            };
        }
        self.lines = lines;
    }

    fn indent_width(&self, level: usize) -> usize {
        let unit = if self.opts.indent == "\t" {
            4
        } else {
            self.opts.indent.chars().count()
        };
        unit * level
    }

    fn width(&self, from: usize, to: usize) -> usize {
        (from..=to)
            .map(|k| self.text(k).chars().count() + usize::from(k > from && self.space[k]) + self.padding[k])
            .sum()
    }

    fn collapse(&mut self, candidates: Vec<usize>) {
        for open in candidates {
            let close = self.pair[open];
            if (open + 1..=close).all(|k| self.brk[k] == Break::None) {
                continue;
            }
            let blocked = (open..=close).any(|k| {
                let t = &self.toks[k];
                t.comment || t.multiline || matches!(t.kind, "line_continuation" | "ERROR") || self.forced[k]
            });
            if blocked {
                continue;
            }
            let li       = self.lines.iter().position(|l| l.first <= open && open <= l.last).unwrap();
            let first    = self.lines[li].first;
            let mut last = close;
            while last + 1 < self.toks.len() && self.brk[last + 1] == Break::None {
                last += 1;
            }
            let width = self.indent_width(self.lines[li].level) + self.width(first, last);
            if width <= self.opts.max_width {
                for k in open + 1..=close {
                    self.brk[k] = Break::None;
                }
                self.compute_lines();
            }
        }
    }

    fn align(&mut self) {
        let mut eqs: Vec<Option<usize>> = Vec::with_capacity(self.lines.len());
        for l in &self.lines {
            let eq = (l.first..=l.last).find(|&k| {
                let t = &self.toks[k];
                t.kind == "=" && t.parent == "assignment" && self.depth[k] == self.depth[l.first]
            });
            let ok = eq.filter(|&e| {
                let stmt = self.toks[e].node.parent().unwrap();
                stmt.start_byte() == self.toks[l.first].start
                    && stmt.end_byte() <= self.toks[l.last].end
                    && !(l.first..=l.last).any(|k| self.toks[k].multiline || self.toks[k].kind == "line_continuation")
            });
            eqs.push(ok);
        }
        let mut li = 0;
        while li < self.lines.len() {
            let mut lj = li;
            while eqs[li].is_some()
                && lj + 1 < self.lines.len()
                && eqs[lj + 1].is_some()
                && self.brk[self.lines[lj + 1].first] == Break::Line
                && self.lines[lj + 1].level == self.lines[li].level
            {
                lj += 1;
            }
            if lj > li {
                let widths: Vec<usize> = (li..=lj).map(|l| self.width(self.lines[l].first, eqs[l].unwrap() - 1)).collect();
                let target             = *widths.iter().max().unwrap();
                for (l, w) in (li..=lj).zip(widths) {
                    self.padding[eqs[l].unwrap()] = target - w;
                }
            }
            li = lj + 1;
        }
    }

    fn reindent(&self, i: usize, new_prefix: &str) -> String {
        let text               = self.text(i);
        let line_start         = self.src[..self.toks[i].start].rfind('\n').map_or(0, |p| p + 1);
        let old_prefix: String = self.src[line_start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let mut parts          = text.split('\n');
        let head               = parts.next().unwrap_or_default();
        let rest: Vec<&str>    = parts.collect();
        let shiftable          = rest.iter().all(|l| l.trim().is_empty() || l.starts_with(old_prefix.as_str()));
        if !shiftable || old_prefix == new_prefix {
            return text.to_string();
        }
        let mut out = head.to_string();
        for l in rest {
            out.push('\n');
            if l.trim().is_empty() {
                out.push_str(l);
            } else {
                out.push_str(new_prefix);
                out.push_str(&l[old_prefix.len()..]);
            }
        }
        out
    }

    fn emit(&self) -> String {
        let mut out = String::with_capacity(self.src.len() + 64);
        for (li, l) in self.lines.iter().enumerate() {
            if li > 0 {
                out.push('\n');
                match self.brk[l.first] {
                    Break::Blank2 if l.level == 0 => out.push_str("\n\n"),
                    Break::Blank | Break::Blank2 => out.push('\n'),
                    _ => {}
                }
            }
            let prefix = self.opts.indent.repeat(l.level);
            out.push_str(&prefix);
            for k in l.first..=l.last {
                if k > l.first {
                    if self.space[k] {
                        out.push(' ');
                    }
                    out.extend(std::iter::repeat_n(' ', self.padding[k]));
                }
                if self.toks[k].multiline && k == l.first && is_docstring(self.toks[k].node) {
                    out.push_str(&self.reindent(k, &prefix));
                } else {
                    out.push_str(self.text(k));
                }
            }
        }
        out.push('\n');
        out
    }
}

fn strip_ws(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().filter(|c| !c.is_whitespace())
}

pub fn format(src: &str, opts: &Options) -> Result<String, String> {
    let tree = parse(src)?;
    if let Some(e) = first_error(tree.root_node()) {
        return Err(format!("syntax error on line {}", e.start_position().row + 1));
    }
    let toks = tokens(&tree, src);
    if toks.is_empty() {
        return Ok(String::new());
    }
    let mut f = Formatter::new(src, opts, toks)?;
    for i in 1..f.toks.len() {
        f.space[i] = f.space_before(i);
    }
    let candidates = f.apply_groups();
    f.apply_chains(&tree);
    f.compute_lines();
    f.collapse(candidates);
    f.align();
    let out = f.emit();

    if !strip_ws(src).eq(strip_ws(&out)) {
        return Err("formatter safety check failed: output changed non-whitespace content".into());
    }
    let formatted = parse(&out)?;
    if first_error(formatted.root_node()).is_some() {
        return Err("formatter safety check failed: output introduced a syntax error".into());
    }
    if formatted.root_node().to_sexp() != tree.root_node().to_sexp() {
        return Err("formatter safety check failed: output changed the code structure".into());
    }
    Ok(out)
}
