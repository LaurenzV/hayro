//! The tag tree, described in Section B.10.2.
//!
//! Tag trees are quad trees where each leaf stores an integer value.
//! Each intermediate node stores the smallest value of all of its children.
//! For example, if a node stores the value 3, it means that all children
//! have a value of 3 or higher. The root node therefore stores the smallest
//! values across all children.

use crate::decode::BitReaderExt;
use hayro_common::bit::BitReader;
use log::warn;

// TODO: Can we change the architecture so that we don't need to reallocate
// for each new tag tree but instead reuse existing allocations?

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub(crate) struct TagNode {
    /// The width of the area covered by the node.
    ///
    /// For leaf nodes, this value is always 1. In some cases, the width might
    /// be 0, in which case the leaf node doesn't actually "exist" and is just
    /// a dummy node.
    width: u32,
    /// The height of the area covered by the node.
    ///
    /// For leaf nodes, this value is always 1. In some cases, the height might
    /// be 0, in which case the leaf node doesn't actually "exist" and is just
    /// a dummy node.
    height: u32,
    /// The actual value stored in the node. Only valid once `initialized`
    /// is set to `true`.
    value: u32,
    /// Whether the node has been fully initialized. The tag tree is not
    /// stored in its complete form in the JP2 file, but is instead built
    /// up incrementally, each packet contributing the information of the
    /// tag tree. The node is therefore only initialized with its actual
    /// value once we cross it the first time.
    initialized: bool,
    /// The level inside the tree. Zero indicates that the given node is
    /// a leaf node, otherwise the level is > 0. The root node has the highest
    /// level.
    level: u16,
    /// The children of the node, some of which might be dummy nodes.
    children: Vec<TagNode>,
}

impl TagNode {
    fn new(width: u32, height: u32, level: u16) -> Self {
        Self {
            width,
            height,
            level,
            value: 0,
            initialized: false,
            children: Vec::with_capacity(4),
        }
    }

    /// The width of the top-left child.
    fn top_left_width(&self) -> u32 {
        u32::min(1 << (self.level - 1), self.width)
    }

    /// The height of the top-left child.
    fn top_left_height(&self) -> u32 {
        u32::min(1 << (self.level - 1), self.height)
    }
}

impl TagNode {
    fn build(width: u32, height: u32, level: u16) -> Self {
        let mut tag = TagNode::new(width, height, level);

        if level == 0 {
            // We reached the leaf node.
            assert!(width <= 1 && height <= 1);

            return tag;
        }

        // Determine the width and height of the top-left child node. Based
        // on this, we can infer the dimensions of all other child nodes.
        let top_left_width = tag.top_left_width();
        let top_left_height = tag.top_left_height();

        let mut push = |node: TagNode| {
            tag.children.push(node);
        };

        // We always push four children, but some nodes might in reality have
        // fewer than that. In this case, the resulting node will simply have
        // a width or height of 0 and we can recognize that it technically
        // doesn't exist.
        push(TagNode::build(top_left_width, top_left_height, level - 1));
        push(TagNode::build(
            width - top_left_width,
            top_left_height,
            level - 1,
        ));
        push(TagNode::build(
            top_left_width,
            height - top_left_height,
            level - 1,
        ));
        push(TagNode::build(
            width - top_left_width,
            height - top_left_height,
            level - 1,
        ));

        tag
    }

    fn read(
        &mut self,
        x: u32,
        y: u32,
        reader: &mut BitReader,
        parent_val: u32,
        max_val: u32,
    ) -> Option<u32> {
        if !self.initialized {
            let mut val = u32::max(parent_val, self.value);

            loop {
                if val >= max_val {
                    break;
                }

                // "Each node has an associated current value, which is
                // initialized to zero (the minimum). A 0 bit in the tag tree
                // means that the minimum (or the value in the case of the
                // highest level) is larger than the current value and a 1 bit
                // means that the minimum (or the value in the case of the
                // highest level) is equal to the current value."
                match reader.read_bits_with_stuffing(1)? {
                    0 => val += 1,
                    1 => {
                        self.initialized = true;
                        break;
                    }
                    _ => unreachable!(),
                }
            }

            self.value = val;
        }

        // Abort early if we already reached the leaf node or the minimum
        // value of all children is too large.
        if self.value >= max_val || self.level == 0 {
            return Some(self.value);
        }

        let top_left_width = self.top_left_width();
        let top_left_height = self.top_left_height();

        let left = x < top_left_width;
        let top = y < top_left_height;

        match (left, top) {
            (true, true) => self.children[0].read(x, y, reader, self.value, max_val),
            (false, true) => {
                self.children[1].read(x - top_left_width, y, reader, self.value, max_val)
            }
            (true, false) => {
                self.children[2].read(x, y - top_left_height, reader, self.value, max_val)
            }
            (false, false) => self.children[3].read(
                x - top_left_width,
                y - top_left_height,
                reader,
                self.value,
                max_val,
            ),
        }
    }
}

#[derive(Clone)]
pub(crate) struct TagTree {
    root: TagNode,
    width: u32,
    height: u32,
}

impl TagTree {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        // Calculate how many levels the tree has in total.
        let level = u32::max(
            width.next_power_of_two().ilog2(),
            height.next_power_of_two().ilog2(),
        );

        Self {
            root: TagNode::build(width, height, level as u16),
            width,
            height,
        }
    }

    pub(crate) fn read(
        &mut self,
        x: u32,
        y: u32,
        reader: &mut BitReader,
        max_val: u32,
    ) -> Option<u32> {
        if x >= self.width || y >= self.height {
            warn!(
                "attempted to read invalid index x: {x}, y: {y} in tag\
            tree with dimensions {}x{}",
                self.width, self.height
            );

            return None;
        }

        self.root.read(x, y, reader, 0, max_val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayro_common::bit::BitWriter;

    impl TagNode {
        fn is_dummy(&self) -> bool {
            self.width == 0 || self.height == 0
        }

        fn real_children(&self) -> usize {
            self.children
                .iter()
                .map(|c| if !c.is_dummy() { 1 } else { 0 })
                .sum()
        }
    }

    /// The example from B.10.2, in its extended form as shown in the
    /// "JPEG2000 Standard for Image compression" book.
    #[test]
    fn tag_tree_1() {
        let mut tree = TagTree::new(6, 3);

        assert_eq!(tree.root.real_children(), 2);
        assert_eq!(tree.root.children[0].real_children(), 4);
        assert_eq!(tree.root.children[0].children[0].real_children(), 4);
        assert_eq!(tree.root.children[0].children[1].real_children(), 4);
        assert_eq!(tree.root.children[0].children[2].real_children(), 2);
        assert_eq!(tree.root.children[0].children[3].real_children(), 2);
        assert_eq!(tree.root.children[1].real_children(), 2);
        assert_eq!(tree.root.children[1].children[0].real_children(), 4);
        assert_eq!(tree.root.children[1].children[2].real_children(), 2);

        let mut buf = vec![0; 3];

        let mut writer = BitWriter::new(&mut buf, 1).unwrap();
        writer.write_bits([
            0, 1, 1, 1, 1, // q3(0, 0)
            0, 0, 1, // q3(1, 0)
            1, 0, 1, // q3(2, 0)
            0, 0, 1, // q3(3, 0)
            1, 0, 1, 1, // q3(4, 0)
        ]);

        let mut reader = BitReader::new(&buf);

        assert_eq!(tree.read(0, 0, &mut reader, u32::MAX).unwrap(), 1);
        assert_eq!(tree.read(1, 0, &mut reader, u32::MAX).unwrap(), 3);
        assert_eq!(tree.read(2, 0, &mut reader, u32::MAX).unwrap(), 2);
        assert_eq!(tree.read(3, 0, &mut reader, u32::MAX).unwrap(), 3);
        assert_eq!(tree.read(4, 0, &mut reader, u32::MAX).unwrap(), 2);
    }

    /// Inclusion tag tree from Table B.5.
    #[test]
    fn tag_tree_2() {
        let mut tree = TagTree::new(3, 2);

        let mut buf = vec![0; 1];

        let mut writer = BitWriter::new(&mut buf, 1).unwrap();
        writer.write_bits([
            1, 1, 1, // Code-block 0, 0 included for the first time (partial
            // inclusion tag tree)
            1, // Code-block 1, 0 included for the first time (partial
            // inclusion tag tree)
            0, // Code-block 2, 0 not yet included (partial tag tree)
            0, // Code-block 0, 1 not yet included
            0, // Code-block 1, 2 not yet included
               // Code-block 2, 1 not yet included (no data needed, already
               // conveyed by partial tag tree for code-block 2, 0)
        ]);

        let mut reader = BitReader::new(&buf);

        let next_layer = 1;

        assert_eq!(tree.read(0, 0, &mut reader, next_layer).unwrap(), 0);
        assert_eq!(tree.read(1, 0, &mut reader, next_layer).unwrap(), 0);
        assert_eq!(tree.read(2, 0, &mut reader, next_layer).unwrap(), 1);
        assert_eq!(tree.read(0, 1, &mut reader, next_layer).unwrap(), 1);
        assert_eq!(tree.read(1, 1, &mut reader, next_layer).unwrap(), 1);
        assert_eq!(tree.read(2, 1, &mut reader, next_layer).unwrap(), 1);
    }
}
