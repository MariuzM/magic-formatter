use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, Tree};

use crate::cursor::SpanCursor;
use crate::language::Analysis;
use crate::locals::Locals;
use crate::text::{LineIndex, utf16_len};

pub const TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "comment",
    "string",
    "number",
    "regexp",
    "operator",
    "decorator",
    "label",
    "boolean",
];

pub const TOKEN_MODIFIERS: &[&str] = &["readonly", "defaultLibrary", "documentation", "declaration"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precedence {
    FirstWins,
    LastWins,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    pub ty:   u32,
    pub mods: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end:   usize,
    pub kind:  Kind,
}

pub struct Highlighter {
    query:      Query,
    kinds:      Vec<Option<Kind>>,
    precedence: Precedence,
}

fn ty(name: &str) -> u32 {
    TOKEN_TYPES.iter().position(|t| *t == name).expect("known token type") as u32
}

fn modifier(name: &str) -> u32 {
    1 << TOKEN_MODIFIERS.iter().position(|m| *m == name).expect("known token modifier")
}

fn kind(t: &str) -> Option<Kind> {
    Some(Kind { ty: ty(t), mods: 0 })
}

fn classify(capture: &str) -> Option<Kind> {
    let head = capture.split('.').next().unwrap_or(capture);
    match capture {
        "comment.documentation" => Some(Kind {
            ty:   ty("comment"),
            mods: modifier("documentation"),
        }),
        "string.regexp" | "string.regex" => kind("regexp"),
        "type.builtin" => Some(Kind {
            ty:   ty("type"),
            mods: modifier("defaultLibrary"),
        }),
        "function.builtin" => Some(Kind {
            ty:   ty("function"),
            mods: modifier("defaultLibrary"),
        }),
        "function.macro" | "constant.macro" => kind("macro"),
        "function.method" | "function.method.call" => kind("method"),
        "variable.parameter" => kind("parameter"),
        "type.parameter" => kind("typeParameter"),
        "variable.member" | "variable.field" => kind("property"),
        "property.declaration" => Some(Kind {
            ty:   ty("property"),
            mods: modifier("declaration"),
        }),
        "boolean" => kind("boolean"),
        "variant" => kind("enumMember"),
        "type.enum" => kind("enum"),
        "variable.builtin" | "constant.builtin" => kind("keyword"),
        "constant" => Some(Kind {
            ty:   ty("variable"),
            mods: modifier("readonly"),
        }),
        _ => match head {
            "comment" => kind("comment"),
            "string" | "character" | "escape" => kind("string"),
            "number" | "float" => kind("number"),
            "type" | "constructor" => kind("type"),
            "function" => kind("function"),
            "property" | "field" => kind("property"),
            "variable" => kind("variable"),
            "keyword" | "conditional" | "repeat" | "include" | "exception" => kind("keyword"),
            "operator" => kind("operator"),
            "attribute" => kind("decorator"),
            "label" => kind("label"),
            "module" | "namespace" => kind("namespace"),
            _ => None,
        },
    }
}

impl Highlighter {
    pub fn new(
        language: &tree_sitter::Language,
        source: &str,
        precedence: Precedence,
        overrides: &[(&str, &str)],
    ) -> Self {
        let query = Query::new(language, source).expect("valid highlights query");
        let kinds = query
            .capture_names()
            .iter()
            .map(|name| match overrides.iter().find(|(from, _)| from == name) {
                Some((_, to)) => kind(to),
                None => classify(name),
            })
            .collect();
        Self { query, kinds, precedence }
    }

    pub fn spans(&self, tree: &Tree, src: &str) -> Vec<Span> {
        let mut raw: Vec<(usize, usize, usize, Option<Kind>)> = Vec::new();
        let mut cursor                                        = QueryCursor::new();
        let mut captures                                      = cursor.captures(&self.query, tree.root_node(), src.as_bytes());
        while let Some((m, i)) = captures.next() {
            let c = m.captures()[*i];
            let n = c.node;
            if n.start_byte() < n.end_byte() {
                raw.push((n.start_byte(), n.end_byte(), m.pattern_index, self.kinds[c.index as usize]));
            }
        }
        raw.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));

        let mut chosen: Vec<Span> = Vec::with_capacity(raw.len());
        let mut i                 = 0;
        while i < raw.len() {
            let mut j = i;
            while j + 1 < raw.len() && raw[j + 1].0 == raw[i].0 && raw[j + 1].1 == raw[i].1 {
                j += 1;
            }
            let winner = match self.precedence {
                Precedence::FirstWins => raw[i].3,
                Precedence::LastWins => raw[j].3,
            };
            if let Some(kind) = winner {
                chosen.push(Span {
                    start: raw[i].0,
                    end: raw[i].1,
                    kind,
                });
            }
            i = j + 1;
        }
        flatten(&chosen)
    }
}

fn flatten(spans: &[Span]) -> Vec<Span> {
    let mut out                       = Vec::with_capacity(spans.len());
    let mut stack: Vec<(usize, Kind)> = Vec::new();
    let mut pos                       = 0;
    let emit                          = |start: usize, end: usize, kind: Kind, out: &mut Vec<Span>| {
        if start < end {
            out.push(Span { start, end, kind });
        }
    };
    for span in spans {
        while let Some(&(end, kind)) = stack.last() {
            if end > span.start {
                break;
            }
            emit(pos, end, kind, &mut out);
            pos = pos.max(end);
            stack.pop();
        }
        if let Some(&(_, kind)) = stack.last() {
            emit(pos, span.start, kind, &mut out);
        }
        pos     = pos.max(span.start);
        let end = stack.last().map_or(span.end, |&(e, _)| span.end.min(e));
        stack.push((end, span.kind));
    }
    while let Some((end, kind)) = stack.pop() {
        emit(pos, end, kind, &mut out);
        pos = pos.max(end);
    }
    out
}

pub fn token_type(name: &str) -> u32 {
    ty(name)
}

pub fn refine_locals(lang: &dyn Analysis, tree: &Tree, src: &str, spans: &mut [Span], locals: &Locals) {
    let (variable, parameter, function) = (ty("variable"), ty("parameter"), ty("function"));
    let kinds                           = lang.identifier_kinds();
    let mut cursor                      = SpanCursor::new(tree);
    for i in 0..spans.len() {
        let span = spans[i];
        if span.kind.ty != variable && span.kind.ty != function {
            continue;
        }
        let node = cursor.seek(span.start, span.end);
        if !(kinds.contains(&node.kind()) && node.start_byte() == span.start && node.end_byte() == span.end) {
            continue;
        }
        if !lang.resolves_locally(node, cursor.parent()) {
            continue;
        }
        let Some(def) = locals.resolve(&src[span.start..span.end], span.start).filter(|d| d.start != span.start) else {
            continue;
        };
        if let Ok(j) = spans.binary_search_by_key(&def.start, |s| s.start)
            && matches!(spans[j].kind.ty, t if t == variable || t == parameter)
        {
            spans[i].kind = spans[j].kind;
        }
    }
}

pub fn mark_enums(spans: &mut [Span], src: &str, is_enum: impl Fn(&str) -> bool) {
    let (from, to) = (ty("type"), ty("enum"));
    for span in spans.iter_mut().filter(|s| s.kind.ty == from) {
        if is_enum(&src[span.start..span.end]) {
            span.kind.ty = to;
        }
    }
}

pub fn mark_functions(spans: &mut [Span], src: &str, is_function: impl Fn(&str) -> bool) {
    let (from, to) = (ty("type"), ty("function"));
    for span in spans.iter_mut().filter(|s| s.kind.ty == from) {
        if is_function(&src[span.start..span.end]) {
            span.kind.ty = to;
        }
    }
}

pub fn encode(spans: &[Span], src: &str, lines: &LineIndex) -> Vec<u32> {
    let mut data      = Vec::with_capacity(spans.len() * 5);
    let mut prev_line = 0u32;
    let mut prev_col  = 0u32;
    let mut cur_line  = usize::MAX;
    let mut col_byte  = 0;
    let mut col_utf16 = 0;
    for span in spans {
        let mut start = span.start;
        while start < span.end {
            let line     = lines.line_of(start);
            let line_end = if line + 1 < lines.line_count() {
                lines.line_start(line + 1) - 1
            } else {
                src.len()
            };
            let end   = span.end.min(line_end);
            let piece = src[start..end].trim_end_matches('\r');
            if !piece.is_empty() {
                if line != cur_line || start < col_byte {
                    cur_line  = line;
                    col_byte  = lines.line_start(line);
                    col_utf16 = 0;
                }
                col_utf16 += utf16_len(&src[col_byte..start]);
                col_byte   = start;
                let col    = col_utf16 as u32;
                let len    = utf16_len(piece) as u32;
                let l   = line as u32;
                let dl  = l - prev_line;
                let dc  = if dl == 0 { col - prev_col } else { col };
                data.extend_from_slice(&[dl, dc, len, span.kind.ty, span.kind.mods]);
                prev_line = l;
                prev_col  = col;
            }
            start = line_end + 1;
        }
    }
    data
}
