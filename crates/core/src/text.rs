use tree_sitter::Point;

#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.bytes().enumerate().filter(|&(_, b)| b == b'\n').map(|(i, _)| i + 1));
        Self { starts }
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    pub fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&s| s <= offset) - 1
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.starts[line.min(self.starts.len() - 1)]
    }

    fn line_content_end(&self, text: &str, line: usize) -> usize {
        let end = self.starts.get(line + 1).map_or(text.len(), |&s| s - 1);
        if end > self.starts[line] && text.as_bytes()[end - 1] == b'\r' { end - 1 } else { end }
    }

    pub fn offset(&self, text: &str, line: u32, utf16_col: u32) -> usize {
        let line = line as usize;
        if line >= self.starts.len() {
            return text.len();
        }
        let start   = self.starts[line];
        let end     = self.line_content_end(text, line);
        let mut col = 0;
        for (i, c) in text[start..end].char_indices() {
            if col >= utf16_col as usize {
                return start + i;
            }
            col += c.len_utf16();
        }
        end
    }

    pub fn position(&self, text: &str, offset: usize) -> (u32, u32) {
        let offset = floor_char_boundary(text, offset.min(text.len()));
        let line   = self.line_of(offset);
        let start  = self.starts[line];
        (line as u32, utf16_len(&text[start..offset]) as u32)
    }

    pub fn point(&self, offset: usize) -> Point {
        let line = self.line_of(offset);
        Point { row: line, column: offset - self.starts[line] }
    }
}

pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

pub fn floor_char_boundary(text: &str, mut offset: usize) -> usize {
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

pub fn point_after(start: Point, inserted: &str) -> Point {
    match inserted.rfind('\n') {
        Some(i) => Point { row: start.row + inserted.matches('\n').count(), column: inserted.len() - i - 1 },
        None => Point { row: start.row, column: start.column + inserted.len() },
    }
}
