#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub(crate) struct TagNode {
    value: u16,
    initialized: bool,
    children: Vec<Box<TagNode>>,
}

impl TagNode {
    fn build(width: u16, height: u16, level: u16) -> Self {
        let mut tag = TagNode::default();

        if level == 0 {
            assert!(width <= 1 && height <= 1);

            return tag;
        }

        let mut push = |node: TagNode| {
            tag.children.push(Box::new(node));
        };

        let x_split = u16::min(1 << (level - 1), width);
        let y_split = u16::min(1 << (level - 1), height);
        let extend_x = width > x_split;
        let extend_y = height > y_split;

        push(TagNode::build(x_split, y_split, level - 1));

        if extend_x {
            push(TagNode::build(width - x_split, y_split, level - 1));
        }

        if extend_y {
            push(TagNode::build(x_split, height - y_split, level - 1));
        }

        if extend_x && extend_y {
            push(TagNode::build(width - x_split, height - y_split, level - 1));
        }

        tag
    }
}

pub(crate) struct TagTree(TagNode);

impl TagTree {
    pub(crate) fn new(width: u16, height: u16) -> Self {
        let level = u32::max(
            width.next_power_of_two().ilog2(),
            height.next_power_of_two().ilog2(),
        );
        Self(TagNode::build(width, height, level as u16))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example from B.10.2.
    #[test]
    fn tag_tree_1() {
        let tree = TagTree::new(6, 3);

        assert_eq!(tree.0.children.len(), 2);
        assert_eq!(tree.0.children[0].children.len(), 4);
        assert_eq!(tree.0.children[0].children[0].children.len(), 4);
        assert_eq!(tree.0.children[0].children[1].children.len(), 4);
        assert_eq!(tree.0.children[0].children[2].children.len(), 2);
        assert_eq!(tree.0.children[0].children[3].children.len(), 2);
        assert_eq!(tree.0.children[1].children.len(), 2);
        assert_eq!(tree.0.children[1].children[0].children.len(), 4);
        assert_eq!(tree.0.children[1].children[1].children.len(), 2);
    }
}
