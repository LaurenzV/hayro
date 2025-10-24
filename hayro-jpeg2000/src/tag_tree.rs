use hayro_common::bit::BitReader;

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub(crate) struct TagNode {
    width: u16,
    height: u16,
    value: u16,
    initialized: bool,
    level: u16,
    children: Vec<Box<TagNode>>,
}

impl TagNode {
    fn new(width: u16, height: u16, level: u16) -> Self {
        Self {
            width,
            height,
            level,
            value: 0,
            initialized: false,
            children: vec![],
        }
    }

    fn x_split(&self) -> u16 {
        u16::min(1 << (self.level - 1), self.width)
    }

    fn y_split(&self) -> u16 {
        u16::min(1 << (self.level - 1), self.height)
    }
}

impl TagNode {
    fn build(width: u16, height: u16, level: u16) -> Self {
        let mut tag = TagNode::new(width, height, level);

        if level == 0 {
            assert!(width <= 1 && height <= 1);

            return tag;
        }

        let x_split = tag.x_split();
        let y_split = tag.y_split();

        let mut push = |node: TagNode| {
            tag.children.push(Box::new(node));
        };

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

    fn read(
        &mut self,
        x: u16,
        y: u16,
        reader: &mut BitReader,
        parent_val: u16,
        max_val: u16,
    ) -> Option<u16> {
        if !self.initialized {
            let mut val = u16::max(parent_val, self.value);
            
            loop {
                if val >= max_val {
                    break;
                }
                
                match reader.read(1)? {
                    0 => val += 1,
                    1 => {
                        self.initialized = true;
                        break;
                    },
                    _ => unreachable!(),
                }
            }

            self.value = val;
        }

        if self.value >= max_val || self.level == 0 {
            return Some(self.value);
        }

        let x_split_idx = self.x_split() - 1;
        let y_split_idx = self.y_split() - 1;

        match (x > x_split_idx, y > y_split_idx) {
            (false, false) => self.children[0].read(x, y, reader, self.value, max_val),
            (true, false) => {
                self.children[1].read(x - x_split_idx, y_split_idx, reader, self.value, max_val)
            }
            (false, true) => {
                let idx = if self.children.len() > 1 { 2 } else { 1 };

                self.children[idx].read(x, y - y_split_idx, reader, self.value, max_val)
            }
            (true, true) => self.children[3].read(
                x - x_split_idx,
                y - y_split_idx,
                reader,
                self.value,
                max_val,
            ),
        }
    }
}

pub(crate) struct TagTree {
    root: TagNode,
    width: u16,
    height: u16,
}

impl TagTree {
    pub(crate) fn new(width: u16, height: u16) -> Self {
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
        x: u16,
        y: u16,
        reader: &mut BitReader,
        max_val: u16,
    ) -> Option<u16> {
        assert!(x < self.width && y < self.height);

        self.root.read(x, y, reader, 0, max_val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayro_common::bit::BitWriter;

    /// The example from B.10.2, in its extended form as shown in the "JPEG2000 Standard for
    /// Image compression" book.
    #[test]
    fn tag_tree_1() {
        let mut tree = TagTree::new(6, 3);

        assert_eq!(tree.root.children.len(), 2);
        assert_eq!(tree.root.children[0].children.len(), 4);
        assert_eq!(tree.root.children[0].children[0].children.len(), 4);
        assert_eq!(tree.root.children[0].children[1].children.len(), 4);
        assert_eq!(tree.root.children[0].children[2].children.len(), 2);
        assert_eq!(tree.root.children[0].children[3].children.len(), 2);
        assert_eq!(tree.root.children[1].children.len(), 2);
        assert_eq!(tree.root.children[1].children[0].children.len(), 4);
        assert_eq!(tree.root.children[1].children[1].children.len(), 2);

        let mut buf = vec![0; 2];

        let mut writer = BitWriter::new(&mut buf, 1).unwrap();
        writer.write_bits([
            0, 1, 1, 1, 1, // q3(0, 0)
            0, 0, 1, // q3(1, 0)
            1, 0, 1, // q3(2, 0)
            0, 0, 1, // q3(3, 0)
            1, 0, 1, 1 // q3(4, 0)
        ]);

        let mut reader = BitReader::new(&buf);

        assert_eq!(tree.read(0, 0, &mut reader, u16::MAX).unwrap(), 1);
        assert_eq!(tree.read(1, 0, &mut reader, u16::MAX).unwrap(), 3);
        assert_eq!(tree.read(2, 0, &mut reader, u16::MAX).unwrap(), 2);
        // assert_eq!(tree.read(3, 0, &mut reader, u16::MAX).unwrap(), 3);
    }

    /// Inclusion tag tree from Table B.5.
    #[test]
    fn tag_tree_2() {
        let mut tree = TagTree::new(3, 2);

        let mut buf = vec![0; 1];

        let mut writer = BitWriter::new(&mut buf, 1).unwrap();
        writer.write_bits([
            1, 1,
            1, // Code-block 0, 0 included for the first time (partial inclusion tag tree)
            1, // Code-block 1, 0 included for the first time (partial inclusion tag tree)
            0, // Code-block 2, 0 not yet included (partial tag tree)
            0, // Code-block 0, 1 not yet included
            0, // Code-block 1, 2 not yet included
               // Code-block 2, 1 not yet included (no data needed, already conveyed by partial tag tree for code-block 2, 0)
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
