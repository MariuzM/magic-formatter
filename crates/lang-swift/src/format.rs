use std::collections::HashSet;

use tree_sitter::{Node, Parser, Tree};

pub struct Options {
    pub indent:    String,
    pub max_width: usize,
}

const ATOMIC: &[&str] = &[
    "line_string_literal",
    "multi_line_string_literal",
    "raw_string_literal",
    "regex_literal",
    "comment",
    "multiline_comment",
    "directive",
    "diagnostic",
    "shebang_line",
];

const CONTAINERS: &[&str] = &[
    "source_file",
    "statements",
    "class_body",
    "protocol_body",
    "enum_class_body",
    "computed_property",
    "willset_didset_block",
    "protocol_property_requirements",
    "switch_statement",
];

const BINARY_PARENTS: &[&str] = &[
    "additive_expression",
    "multiplicative_expression",
    "equality_expression",
    "conjunction_expression",
    "disjunction_expression",
    "nil_coalescing_expression",
    "bitwise_operation",
    "assignment",
    "protocol_composition_type",
    "equality_constraint",
    "ternary_expression",
];

const ALIGN_PARENTS: &[&str] = &["property_declaration", "assignment", "typealias_declaration"];

const BODY_WRAPPERS: &[&str] =
    &["function_body", "class_body", "protocol_body", "enum_class_body", "computed_property"];

const BLOCK_OWNERS: &[&str] = &[
    "if_statement",
    "guard_statement",
    "while_statement",
    "for_statement",
    "switch_statement",
    "repeat_while_statement",
    "do_statement",
    "catch_block",
    "function_declaration",
    "init_declaration",
    "deinit_declaration",
    "subscript_declaration",
    "class_declaration",
    "protocol_declaration",
    "property_declaration",
];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Break {
    None,
    Line,
    Blank,
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
        let atom = node.is_error() || ATOMIC.contains(&node.kind());
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
                comment: matches!(kind, "comment" | "multiline_comment"),
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
    let mut out = Vec::with_capacity(toks.len());
    let mut pos = 0;
    for (i, tok) in toks.iter().enumerate() {
        let node = toks[i].node;
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
                node,
                start,
                end: k,
                kind: if &src[start..k] == ";" { ";" } else { "ERROR" },
                parent: "",
                named: false,
                comment: false,
                multiline: false,
            });
        }
        pos = tok.end;
    }
    let mut toks = toks;
    if out.is_empty() {
        return toks;
    }
    toks.append(&mut out);
    toks.sort_by_key(|t| t.start);
    toks
}

fn error_count(node: Node) -> usize {
    if node.is_error() || node.is_missing() {
        return 1;
    }
    if !node.has_error() {
        return 0;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).map(error_count).sum()
}

fn parse(src: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_swift::LANGUAGE.into()).map_err(|e| e.to_string())?;
    parser.parse(src, None).ok_or_else(|| "parser failed".into())
}

fn is_open(kind: &str) -> bool {
    matches!(kind, "(" | "[" | "{")
}

fn is_close(kind: &str) -> bool {
    matches!(kind, ")" | "]" | "}")
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
    lines:   Vec<Line>,
    padding: Vec<usize>,
}

impl<'a, 't> Formatter<'a, 't> {
    fn new(src: &'a str, opts: &'a Options, toks: Vec<Tok<'t>>) -> Result<Self, String> {
        let n         = toks.len();
        let mut brk   = vec![Break::None; n];
        let mut ws    = vec![false; n];
        let mut pair  = vec![usize::MAX; n];
        let mut stack = Vec::new();
        for i in 0..n {
            if i > 0 {
                let gap  = &src[toks[i - 1].end..toks[i].start];
                let nl   = gap.matches('\n').count();
                ws[i]    = !gap.is_empty();
                brk[i]   = match nl {
                    0 => Break::None,
                    1 => Break::Line,
                    _ => Break::Blank,
                };
            }
            if is_open(toks[i].kind) {
                stack.push(i);
            } else if is_close(toks[i].kind) {
                let o = stack.pop().ok_or("unbalanced brackets")?;
                pair[o] = i;
                pair[i] = o;
            }
        }
        if !stack.is_empty() {
            return Err("unbalanced brackets".into());
        }
        Ok(Self {
            src,
            opts,
            toks,
            brk,
            ws,
            space: vec![false; n],
            forced: vec![false; n],
            pair,
            lines: Vec::new(),
            padding: vec![0; n],
        })
    }

    fn text(&self, i: usize) -> &'a str {
        &self.src[self.toks[i].start..self.toks[i].end]
    }

    fn is_line_comment(&self, i: usize) -> bool {
        self.toks[i].kind == "comment" && self.text(i).starts_with("//")
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
            if !matches!(self.toks[open].kind, "(" | "[") {
                continue;
            }
            let close  = self.pair[open];
            let commas = self.top_level_commas(open);
            if commas.is_empty() || (open..=close).any(|k| self.toks[k].kind == "ERROR") {
                continue;
            }
            let trailing   = self.toks[self.last_code_before(close)].kind == ",";
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
            "navigation_expression" => {
                seen.insert(node.id());
                let mut cursor = node.walk();
                if let Some(suffix) = node.children(&mut cursor).find(|c| c.kind() == "navigation_suffix")
                    && let Some(dot) = suffix.child(0).filter(|d| d.kind() == ".")
                {
                    dots.push(dot.start_byte());
                }
                if let Some(target) = node.child(0) {
                    self.chain_dots(target, dots, seen);
                }
            }
            "call_expression" => {
                if let Some(callee) = node.child(0) {
                    self.chain_dots(callee, dots, seen);
                }
            }
            "postfix_expression" => {
                if let Some(target) = node.child_by_field_name("target") {
                    self.chain_dots(target, dots, seen);
                }
            }
            _ => {}
        }
    }

    fn apply_chains(&mut self, tree: &Tree) {
        let by_start: std::collections::HashMap<usize, usize> =
            self.toks.iter().enumerate().filter(|(_, t)| t.kind == ".").map(|(i, t)| (t.start, i)).collect();
        let mut seen   = HashSet::new();
        let mut cursor = tree.walk();
        'walk: loop {
            let node = cursor.node();
            if node.kind() == "navigation_expression" && !seen.contains(&node.id()) {
                let mut dots = Vec::new();
                self.chain_dots(node, &mut dots, &mut seen);
                dots.sort_unstable();
                let idx: Vec<usize> = dots.iter().filter_map(|d| by_start.get(d).copied()).collect();
                if idx.len() == dots.len() && idx.first().is_some_and(|&first| self.brk[first] != Break::None) {
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
        !t.named && !matches!(t.kind, "(" | ")" | "[" | "]" | "{" | "}" | ",")
            && (matches!(t.kind, "=" | "->") || BINARY_PARENTS.contains(&t.parent))
    }

    fn is_chain_dot(&self, i: usize) -> bool {
        self.toks[i].kind == "." && self.toks[i].parent == "navigation_suffix"
    }

    fn is_generic_angle(&self, i: usize) -> bool {
        matches!(self.toks[i].kind, "<" | ">") && matches!(self.toks[i].parent, "type_arguments" | "type_parameters")
    }

    fn space_before(&self, i: usize) -> bool {
        let a  = &self.toks[i - 1];
        let b  = &self.toks[i];
        let ws = self.ws[i];
        if b.comment {
            return true;
        }
        if a.comment || a.kind == "ERROR" || b.kind == "ERROR" {
            return ws;
        }
        if matches!(b.kind, "," | ";" | ")" | "]") || matches!(a.kind, "(" | "[") {
            return false;
        }
        if a.kind == "," || b.kind == "{" {
            return true;
        }
        if a.kind == "{" {
            return b.kind != "}";
        }
        if b.kind == "}" {
            return true;
        }
        if b.kind == ":" && !b.named {
            return b.parent == "ternary_expression";
        }
        if a.kind == ":" && !a.named {
            if a.parent == "value_argument" && a.node.parent().and_then(|p| p.child_by_field_name("value")).is_none() {
                return ws;
            }
            return true;
        }
        if self.is_binary(i - 1) || self.is_binary(i) {
            return true;
        }
        if !a.named && matches!(a.kind, "." | "@" | "#" | "\\") {
            return false;
        }
        if b.kind == "." && b.parent == "navigation_suffix" {
            return false;
        }
        if self.is_generic_angle(i) || a.kind == "<" && self.is_generic_angle(i - 1) {
            return false;
        }
        ws
    }

    fn starts_container_child(&self, i: usize) -> bool {
        let start    = self.toks[i].start;
        let mut node = self.toks[i].node;
        while let Some(parent) = node.parent() {
            if CONTAINERS.contains(&parent.kind()) {
                return true;
            }
            if parent.start_byte() != start {
                return false;
            }
            node = parent;
        }
        false
    }

    fn starts_node(&self, i: usize, kind: &str) -> bool {
        let start    = self.toks[i].start;
        let mut node = self.toks[i].node;
        while let Some(parent) = node.parent() {
            if parent.start_byte() != start {
                return false;
            }
            if parent.kind() == kind {
                return true;
            }
            node = parent;
        }
        false
    }

    fn ends_attribute(&self, i: usize) -> bool {
        let end      = self.toks[i].end;
        let mut node = self.toks[i].node;
        while let Some(parent) = node.parent() {
            if parent.kind() == "attribute" {
                return true;
            }
            if parent.end_byte() != end {
                return false;
            }
            node = parent;
        }
        false
    }

    fn is_fresh(&self, i: usize, top: Option<usize>) -> bool {
        let t = &self.toks[i];
        if matches!(t.kind, "directive" | "{" | "else" | "catch_keyword" | "statement_label") {
            return true;
        }
        let Some(p) = (0..i).rev().find(|&j| !self.toks[j].comment) else { return true };
        let prev = &self.toks[p];
        if matches!(prev.kind, "{" | "(" | "[" | ";" | "directive") {
            return true;
        }
        if prev.kind == "," && top.is_some_and(|o| matches!(self.toks[o].kind, "(" | "[")) {
            return true;
        }
        if prev.kind == "in" && prev.parent == "lambda_literal" {
            return true;
        }
        if self.ends_attribute(p) {
            return true;
        }
        self.starts_container_child(i)
    }

    fn anchor_line(&self, open: usize, line_of: &[usize]) -> usize {
        if self.toks[open].kind != "{" {
            return line_of[open];
        }
        let mut owner = self.toks[open].node.parent();
        if owner.is_some_and(|o| BODY_WRAPPERS.contains(&o.kind())) {
            owner = owner.and_then(|o| o.parent());
        }
        match owner.filter(|o| BLOCK_OWNERS.contains(&o.kind())) {
            Some(o) => line_of[self.toks.partition_point(|t| t.start <= o.start_byte()).saturating_sub(1)],
            None => line_of[open],
        }
    }

    fn compute_lines(&mut self) {
        let n         = self.toks.len();
        let mut lines = Vec::new();
        let mut first = 0;
        for i in 1..=n {
            if i == n || self.brk[i] != Break::None || i > 0 && self.is_line_comment(i - 1) {
                lines.push(Line { first, last: i - 1, level: 0 });
                first = i;
            }
        }
        for i in 1..n {
            if self.is_line_comment(i - 1) && self.brk[i] == Break::None {
                self.brk[i] = Break::Line;
            }
        }

        let mut line_of = vec![0; n];
        for (li, l) in lines.iter().enumerate() {
            for k in l.first..=l.last {
                line_of[k] = li;
            }
        }

        let mut stack: Vec<usize> = Vec::new();
        let mut anchor            = vec![0; n];
        let mut pp_at             = vec![0; n];
        let mut pp                = 0usize;
        let mut deferred          = Vec::new();
        let mut prev_code         = None;
        for li in 0..lines.len() {
            let (lf, ll) = (lines[li].first, lines[li].last);
            let code     = (lf..=ll).find(|&k| !self.toks[k].comment);
            match code {
                None => deferred.push(li),
                Some(t) => {
                    let only_closers = |p: usize| {
                        (lines[p].first..=lines[p].last).all(|k| is_close(self.toks[k].kind) || self.toks[k].comment)
                    };
                    let follows_chain = self.is_chain_dot(t)
                        && prev_code.is_some_and(|p: usize| self.is_chain_dot(lines[p].first) || only_closers(p));
                    let top       = stack.last().copied();
                    let directive = (self.toks[t].kind == "directive").then(|| self.text(t));
                    if directive.is_some_and(|d| d.starts_with("#endif")) {
                        pp = pp.saturating_sub(1);
                    }
                    let mut offset = pp.saturating_sub(top.map_or(0, |o| pp_at[o]));
                    if directive.is_some_and(|d| d.starts_with("#else")) {
                        offset = offset.saturating_sub(1);
                    }
                    let level = if is_close(self.toks[t].kind) {
                        lines[anchor[self.pair[t]]].level
                    } else if follows_chain {
                        lines[prev_code.unwrap()].level
                    } else {
                        let base    = top.map_or(0, |o| lines[anchor[o]].level + 1) + offset;
                        let hanging = top.is_some_and(|o| self.toks[o].kind != "{" && self.brk[o + 1] == Break::None);
                        if self.starts_node(t, "switch_entry") && top.is_some() {
                            base.saturating_sub(1)
                        } else if hanging || self.is_fresh(t, top) {
                            base
                        } else {
                            base + 1
                        }
                    };
                    if directive.is_some_and(|d| d.starts_with("#if")) {
                        pp += 1;
                    }
                    lines[li].level = level;
                    prev_code = Some(li);
                }
            }
            for k in lf..=ll {
                if is_open(self.toks[k].kind) {
                    anchor[k] = self.anchor_line(k, &line_of);
                    pp_at[k] = pp;
                    stack.push(k);
                } else if is_close(self.toks[k].kind) {
                    stack.pop();
                }
            }
        }
        for li in deferred.into_iter().rev() {
            let next = (li + 1..lines.len()).find(|&l| (lines[l].first..=lines[l].last).any(|k| !self.toks[k].comment));
            lines[li].level = match next {
                Some(l) if is_close(self.toks[lines[l].first].kind) => lines[l].level + 1,
                Some(l) => lines[l].level,
                None => 0,
            };
        }
        self.lines = lines;
    }

    fn indent_width(&self, level: usize) -> usize {
        let unit = if self.opts.indent == "\t" { 4 } else { self.opts.indent.chars().count() };
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
                t.comment || t.multiline || t.kind == "{" || t.kind == "directive" || self.forced[k]
            });
            if blocked {
                continue;
            }
            let li    = self.lines.iter().position(|l| l.first <= open && open <= l.last).unwrap();
            let first = self.lines[li].first;
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
                t.kind == "=" && !t.named && ALIGN_PARENTS.contains(&t.parent)
            });
            let ok = eq.filter(|&e| {
                let stmt = self.toks[e].node.parent().unwrap();
                stmt.start_byte() == self.toks[l.first].start
                    && stmt.end_byte() <= self.toks[l.last].end
                    && !(l.first..=l.last).any(|k| self.toks[k].multiline)
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
                let widths: Vec<usize> =
                    (li..=lj).map(|l| self.width(self.lines[l].first, eqs[l].unwrap() - 1)).collect();
                let target = *widths.iter().max().unwrap();
                for (l, w) in (li..=lj).zip(widths) {
                    self.padding[eqs[l].unwrap()] = target - w;
                }
            }
            li = lj + 1;
        }
    }

    fn reindent(&self, i: usize, new_prefix: &str) -> String {
        let text       = self.text(i);
        let line_start = self.src[..self.toks[i].start].rfind('\n').map_or(0, |p| p + 1);
        let old_prefix: String =
            self.src[line_start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let mut parts = text.split('\n');
        let head      = parts.next().unwrap_or_default();
        let rest: Vec<&str> = parts.collect();
        let shiftable = rest.iter().all(|l| l.trim().is_empty() || l.starts_with(old_prefix.as_str()));
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
                if self.brk[l.first] == Break::Blank {
                    out.push('\n');
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
                if self.toks[k].multiline {
                    out.push_str(&self.reindent(k, &prefix));
                } else {
                    out.push_str(self.text(k));
                }
            }
        }
        let mut result: String = out.split('\n').map(str::trim_end).collect::<Vec<_>>().join("\n");
        result.push('\n');
        result
    }
}

fn strip_ws(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().filter(|c| !c.is_whitespace())
}

pub fn format(src: &str, opts: &Options) -> Result<String, String> {
    let tree   = parse(src)?;
    let errors = error_count(tree.root_node());
    let toks   = tokens(&tree, src);
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
    if error_count(parse(&out)?.root_node()) > errors {
        return Err("formatter safety check failed: output introduced a syntax error".into());
    }
    Ok(out)
}
