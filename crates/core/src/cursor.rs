use tree_sitter::{Node, Tree, TreeCursor};

pub struct SpanCursor<'t> {
    cursor: TreeCursor<'t>,
    stack:  Vec<Node<'t>>,
}

impl<'t> SpanCursor<'t> {
    pub fn new(tree: &'t Tree) -> Self {
        Self {
            cursor: tree.walk(),
            stack:  Vec::new(),
        }
    }

    pub fn seek(&mut self, start: usize, end: usize) -> Node<'t> {
        loop {
            let node = self.cursor.node();
            if node.start_byte() <= start && end <= node.end_byte() {
                break;
            }
            if node.end_byte() <= start && self.cursor.goto_next_sibling() {
                continue;
            }
            if !self.cursor.goto_parent() {
                break;
            }
            self.stack.pop();
        }
        loop {
            let parent = self.cursor.node();
            if !self.cursor.goto_first_child() {
                return parent;
            }
            self.stack.push(parent);
            loop {
                let child = self.cursor.node();
                if child.start_byte() <= start && end <= child.end_byte() {
                    break;
                }
                if child.start_byte() > start || !self.cursor.goto_next_sibling() {
                    self.cursor.goto_parent();
                    self.stack.pop();
                    return parent;
                }
            }
        }
    }

    pub fn parent(&self) -> Option<Node<'t>> {
        self.stack.last().copied()
    }

    pub fn ancestors(&self) -> &[Node<'t>] {
        &self.stack
    }
}
