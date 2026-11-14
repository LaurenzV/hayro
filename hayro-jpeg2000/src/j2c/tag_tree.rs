use alloc::vec::Vec;

use crate::reader::BitReader;

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub(crate) struct TagNode {
    width: u32,
    height: u32,
    value: u32,
    initialized: bool,
    level: u16,
    children: [usize; 4],
}

impl TagNode {
    fn new(width: u32, height: u32, level: u16) -> Self {
        Self {
            width,
            height,
            level,
            value: 0,
            initialized: false,
            children: [usize::MAX, usize::MAX, usize::MAX, usize::MAX],
        }
    }

    fn top_left_width(&self) -> u32 {
        u32::min(1 << (self.level - 1), self.width)
    }

    fn top_left_height(&self) -> u32 {
        u32::min(1 << (self.level - 1), self.height)
    }
}

impl TagNode {
    fn build(width: u32, height: u32, level: u16, nodes: &mut Vec<Self>) -> Self {
        let mut tag = Self::new(width, height, level);

        if level == 0 {
            assert!(width <= 1 && height <= 1);

            return tag;
        }

        let top_left_width = tag.top_left_width();
        let top_left_height = tag.top_left_height();

        let mut push = |node: Self, child_idx: usize, nodes: &mut Vec<Self>| {
            if node.width > 0 && node.height > 0 {
                let node_idx = nodes.len();
                nodes.push(node);
                tag.children[child_idx] = node_idx;
            }
        };

        let n1 = Self::build(top_left_width, top_left_height, level - 1, nodes);
        push(n1, 0, nodes);

        let n2 = Self::build(width - top_left_width, top_left_height, level - 1, nodes);
        push(n2, 1, nodes);

        let n3 = Self::build(top_left_width, height - top_left_height, level - 1, nodes);
        push(n3, 2, nodes);

        let n4 = Self::build(
            width - top_left_width,
            height - top_left_height,
            level - 1,
            nodes,
        );
        push(n4, 3, nodes);

        tag
    }
}

fn read_tag_node(
    node_idx: usize,
    x: u32,
    y: u32,
    reader: &mut BitReader<'_>,
    parent_val: u32,
    max_val: u32,
    nodes: &mut [TagNode],
) -> Option<u32> {
    let node = &mut nodes[node_idx];

    if !node.initialized {
        let mut val = u32::max(parent_val, node.value);

        loop {
            if val >= max_val {
                break;
            }

            match reader.read_bits_with_stuffing(1)? {
                0 => val += 1,
                1 => {
                    node.initialized = true;
                    break;
                }
                _ => unreachable!(),
            }
        }

        node.value = val;
    }

    if node.value >= max_val || node.level == 0 {
        return Some(node.value);
    }

    let top_left_width = node.top_left_width();
    let top_left_height = node.top_left_height();

    let left = x < top_left_width;
    let top = y < top_left_height;

    match (left, top) {
        (true, true) => read_tag_node(node.children[0], x, y, reader, node.value, max_val, nodes),
        (false, true) => read_tag_node(
            node.children[1],
            x - top_left_width,
            y,
            reader,
            node.value,
            max_val,
            nodes,
        ),
        (true, false) => read_tag_node(
            node.children[2],
            x,
            y - top_left_height,
            reader,
            node.value,
            max_val,
            nodes,
        ),
        (false, false) => read_tag_node(
            node.children[3],
            x - top_left_width,
            y - top_left_height,
            reader,
            node.value,
            max_val,
            nodes,
        ),
    }
}

#[derive(Copy, Clone)]
pub(crate) struct TagTree {
    root: usize,
    width: u32,
    height: u32,
}

impl TagTree {
    pub(crate) fn new(width: u32, height: u32, nodes: &mut Vec<TagNode>) -> Self {
        let level = u32::max(
            width.next_power_of_two().ilog2(),
            height.next_power_of_two().ilog2(),
        );

        let node = TagNode::build(width, height, level as u16, nodes);
        let idx = nodes.len();
        nodes.push(node);

        Self {
            root: idx,
            width,
            height,
        }
    }

    pub(crate) fn read(
        &mut self,
        x: u32,
        y: u32,
        reader: &mut BitReader<'_>,
        max_val: u32,
        nodes: &mut [TagNode],
    ) -> Option<u32> {
        debug_assert!(x < self.width && y < self.height);

        read_tag_node(self.root, x, y, reader, 0, max_val, nodes)
    }
}
