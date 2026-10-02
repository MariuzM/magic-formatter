use tree_sitter::{Node, Parser, Tree};

pub struct Options {
    pub indent:    String,
    pub max_width: usize,
}

const ATOMIC: &[&str] = &["string", "quoted_key", "comment"];

const STATEMENT_PARENTS: &[&str] = &["document", "table", "table_array_element"];

#[derive(Clone, Copy, PartialEq, Eq)]
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
        let text = &src[node.start_byte()..node.end_byte()];
        if (atom || node.child_count() == 0) && !text.trim().is_empty() {
            let kind = node.kind();
            out.push(Tok {
                node,
                start: node.start_byte(),
                end: node.end_byte(),
                kind,
                parent: node.parent().map_or("", |p| p.kind()),
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
    out
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
    parser.set_language(&tree_sitter_toml_ng::LANGUAGE.into()).map_err(|e| e.to_string())?;
    parser.parse(src, None).ok_or_else(|| "parser failed".into())
}

fn is_open(kind: &str) -> bool {
    matches!(kind, "[" | "[[" | "{")
}

fn is_close(kind: &str) -> bool {
    matches!(kind, "]" | "]]" | "}")
}

struct Formatter<'a, 't> {
    src:     &'a str,
    opts:    &'a Options,
    toks:    Vec<Tok<'t>>,
    brk:     Vec<Break>,
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
        let mut pair  = vec![usize::MAX; n];
        let mut stack = Vec::new();
        for i in 0..n {
            if i > 0 {
                brk[i] = match src[toks[i - 1].end..toks[i].start].matches('\n').count() {
                    0 => Break::None,
                    1 => Break::Line,
                    _ => Break::Blank,
                };
            }
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
        Ok(Self {
            src,
            opts,
            toks,
            brk,
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

    fn space_before(&self, i: usize) -> bool {
        let a = &self.toks[i - 1];
        let b = &self.toks[i];
        if b.comment || a.kind == "=" || b.kind == "=" {
            return true;
        }
        if matches!(b.kind, "," | "." | "]" | "]]") || matches!(a.kind, "." | "[" | "[[") {
            return false;
        }
        if a.kind == "{" {
            return b.kind != "}";
        }
        true
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
        for open in (0..self.toks.len()).rev() {
            if self.toks[open].kind != "[" || self.toks[open].parent != "array" {
                continue;
            }
            let close  = self.pair[open];
            let commas = self.top_level_commas(open);
            if commas.is_empty() {
                continue;
            }
            let trailing   = self.toks[self.last_code_before(close)].kind == ",";
            let multiline  = (open + 1..=close).any(|k| self.brk[k] != Break::None);
            let first_next = self.brk[open + 1] != Break::None;
            let nested     = (open + 1..close).any(|k| self.forced[k]);
            if trailing || nested || multiline && first_next {
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
        candidates.reverse();
        candidates
    }

    fn compute_lines(&mut self) {
        let n = self.toks.len();
        for i in 1..n {
            if self.toks[i - 1].comment && self.brk[i] == Break::None {
                self.brk[i] = Break::Line;
            }
        }
        let mut lines = Vec::new();
        let mut first = 0;
        for i in 1..=n {
            if i == n || self.brk[i] != Break::None {
                lines.push(Line {
                    first,
                    last: i - 1,
                    level: 0,
                });
                first = i;
            }
        }

        let mut stack: Vec<usize> = Vec::new();
        let mut anchor            = vec![0; n];
        let mut deferred          = Vec::new();
        for li in 0..lines.len() {
            let (lf, ll) = (lines[li].first, lines[li].last);
            match (lf..=ll).find(|&k| !self.toks[k].comment) {
                None => deferred.push(li),
                Some(t) => {
                    lines[li].level = if is_close(self.toks[t].kind) {
                        lines[anchor[self.pair[t]]].level
                    } else {
                        stack.last().map_or(0, |&o| lines[anchor[o]].level + 1)
                    };
                }
            }
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
            let blocked = (open..=close).any(|k| self.toks[k].comment || self.toks[k].multiline || self.forced[k]);
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
            let eq = (l.first..=l.last).find(|&k| self.toks[k].kind == "=");
            let ok = eq.filter(|&e| {
                let node        = self.toks[e].node;
                let Some(pair)  = node.parent() else { return false };
                let Some(value) = node.next_sibling() else { return false };
                let opener      = value.kind() == "array" && self.toks[l.last].start == value.start_byte();
                pair.parent().is_some_and(|p| STATEMENT_PARENTS.contains(&p.kind()))
                    && pair.start_byte() == self.toks[l.first].start
                    && (value.end_byte() <= self.toks[l.last].end || opener)
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
                let widths: Vec<usize> = (li..=lj).map(|l| self.width(self.lines[l].first, eqs[l].unwrap() - 1)).collect();
                let target             = *widths.iter().max().unwrap();
                for (l, w) in (li..=lj).zip(widths) {
                    self.padding[eqs[l].unwrap()] = target - w;
                }
            }
            li = lj + 1;
        }
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
            out.push_str(&self.opts.indent.repeat(l.level));
            for k in l.first..=l.last {
                if k > l.first {
                    if self.space[k] {
                        out.push(' ');
                    }
                    out.extend(std::iter::repeat_n(' ', self.padding[k]));
                }
                out.push_str(self.text(k));
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
    f.compute_lines();
    f.collapse(candidates);
    f.align();
    let out = f.emit();

    if !strip_ws(src).eq(strip_ws(&out)) {
        return Err("formatter safety check failed: output changed non-whitespace content".into());
    }
    if first_error(parse(&out)?.root_node()).is_some() {
        return Err("formatter safety check failed: output introduced a syntax error".into());
    }
    Ok(out)
}
