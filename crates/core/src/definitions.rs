use tree_sitter::{Node, Tree};

fn base_name(mut node: Node) -> Node {
    loop {
        let next = ["name", "type", "bound_identifier"].iter().find_map(|f| node.child_by_field_name(f)).or_else(|| {
            let mut cursor = node.walk();
            (node.kind() == "user_type")
                .then(|| node.named_children(&mut cursor).filter(|c| c.kind() == "type_identifier").last())
                .flatten()
        });
        match next {
            Some(n) => node = n,
            None => return node,
        }
    }
}

pub fn find(tree: &Tree, src: &str, name: &str, kinds: &[(&str, &str)]) -> Vec<(usize, usize)> {
    let bytes      = src.as_bytes();
    let mut out    = Vec::new();
    let mut cursor = tree.walk();

    'walk: loop {
        let node = cursor.node();
        for &(_, field) in kinds.iter().filter(|(k, _)| *k == node.kind()) {
            let mut c            = node.walk();
            let named: Vec<Node> = if field.is_empty() {
                node.named_children(&mut c).find(|n| n.child_count() == 0).into_iter().collect()
            } else {
                node.children_by_field_name(field, &mut c).collect()
            };
            for target in named.into_iter().map(base_name) {
                let hit = (target.start_byte(), target.end_byte());
                if target.utf8_text(bytes).is_ok_and(|t| t == name) && !out.contains(&hit) {
                    out.push(hit);
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
    out
}
