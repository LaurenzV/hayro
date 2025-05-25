use once_cell::sync::Lazy;
use crate::filter::jbig2::{HuffmanLine, HuffmanTable, Jbig2Error};

pub fn get_standard_table(number: u32) -> Result<HuffmanTable, Jbig2Error> {
    if number == 0 || number > 15 {
        return Err(Jbig2Error::new("invalid standard table"));
    }   else {
        Ok(Lazy::force(&STANDARD_TABLES[number as usize - 1]).clone())
    }
}

static STANDARD_TABLES: [Lazy<HuffmanTable>; 15] = [
    Lazy::new(|| build_standard_table(1)),
    Lazy::new(|| build_standard_table(2)),
    Lazy::new(|| build_standard_table(3)),
    Lazy::new(|| build_standard_table(4)),
    Lazy::new(|| build_standard_table(5)),
    Lazy::new(|| build_standard_table(6)),
    Lazy::new(|| build_standard_table(7)),
    Lazy::new(|| build_standard_table(8)),
    Lazy::new(|| build_standard_table(9)),
    Lazy::new(|| build_standard_table(10)),
    Lazy::new(|| build_standard_table(11)),
    Lazy::new(|| build_standard_table(12)),
    Lazy::new(|| build_standard_table(13)),
    Lazy::new(|| build_standard_table(14)),
    Lazy::new(|| build_standard_table(15)),
];

fn build_standard_table(number: u32) -> HuffmanTable {
    // Annex B.5 Standard Huffman tables
    let lines_data: Vec<Vec<i32>> = match number {
        1 => vec![
            vec![0, 1, 4, 0x0],
            vec![16, 2, 8, 0x2],
            vec![272, 3, 16, 0x6],
            vec![65808, 3, 32, 0x7], // upper
        ],
        2 => vec![
            vec![0, 1, 0, 0x0],
            vec![1, 2, 0, 0x2],
            vec![2, 3, 0, 0x6],
            vec![3, 4, 3, 0xe],
            vec![11, 5, 6, 0x1e],
            vec![75, 6, 32, 0x3e], // upper
            vec![6, 0x3f],         // OOB
        ],
        3 => vec![
            vec![-256, 8, 8, 0xfe],
            vec![0, 1, 0, 0x0],
            vec![1, 2, 0, 0x2],
            vec![2, 3, 0, 0x6],
            vec![3, 4, 3, 0xe],
            vec![11, 5, 6, 0x1e],
            vec![-257, 8, 32, 0xff, -1], // lower (using -1 as marker)
            vec![75, 7, 32, 0x7e],       // upper
            vec![6, 0x3e],               // OOB
        ],
        4 => vec![
            vec![1, 1, 0, 0x0],
            vec![2, 2, 0, 0x2],
            vec![3, 3, 0, 0x6],
            vec![4, 4, 3, 0xe],
            vec![12, 5, 6, 0x1e],
            vec![76, 5, 32, 0x1f], // upper
        ],
        5 => vec![
            vec![-255, 7, 8, 0x7e],
            vec![1, 1, 0, 0x0],
            vec![2, 2, 0, 0x2],
            vec![3, 3, 0, 0x6],
            vec![4, 4, 3, 0xe],
            vec![12, 5, 6, 0x1e],
            vec![-256, 7, 32, 0x7f, -1], // lower
            vec![76, 6, 32, 0x3e],       // upper
        ],
        6 => vec![
            vec![-2048, 5, 10, 0x1c],
            vec![-1024, 4, 9, 0x8],
            vec![-512, 4, 8, 0x9],
            vec![-256, 4, 7, 0xa],
            vec![-128, 5, 6, 0x1d],
            vec![-64, 5, 5, 0x1e],
            vec![-32, 4, 5, 0xb],
            vec![0, 2, 7, 0x0],
            vec![128, 3, 7, 0x2],
            vec![256, 3, 8, 0x3],
            vec![512, 4, 9, 0xc],
            vec![1024, 4, 10, 0xd],
            vec![-2049, 6, 32, 0x3e, -1], // lower
            vec![2048, 6, 32, 0x3f],      // upper
        ],
        7 => vec![
            vec![-1024, 4, 9, 0x8],
            vec![-512, 3, 8, 0x0],
            vec![-256, 4, 7, 0x9],
            vec![-128, 5, 6, 0x1a],
            vec![-64, 5, 5, 0x1b],
            vec![-32, 4, 5, 0xa],
            vec![0, 4, 5, 0xb],
            vec![32, 5, 5, 0x1c],
            vec![64, 5, 6, 0x1d],
            vec![128, 4, 7, 0xc],
            vec![256, 3, 8, 0x1],
            vec![512, 3, 9, 0x2],
            vec![1024, 3, 10, 0x3],
            vec![-1025, 5, 32, 0x1e, -1], // lower
            vec![2048, 5, 32, 0x1f],      // upper
        ],
        8 => vec![
            vec![-15, 8, 3, 0xfc],
            vec![-7, 9, 1, 0x1fc],
            vec![-5, 8, 1, 0xfd],
            vec![-3, 9, 0, 0x1fd],
            vec![-2, 7, 0, 0x7c],
            vec![-1, 4, 0, 0xa],
            vec![0, 2, 1, 0x0],
            vec![2, 5, 0, 0x1a],
            vec![3, 6, 0, 0x3a],
            vec![4, 3, 4, 0x4],
            vec![20, 6, 1, 0x3b],
            vec![22, 4, 4, 0xb],
            vec![38, 4, 5, 0xc],
            vec![70, 5, 6, 0x1b],
            vec![134, 5, 7, 0x1c],
            vec![262, 6, 7, 0x3c],
            vec![390, 7, 8, 0x7d],
            vec![646, 6, 10, 0x3d],
            vec![-16, 9, 32, 0x1fe, -1], // lower
            vec![1670, 9, 32, 0x1ff],    // upper
            vec![2, 0x1],                // OOB
        ],
        9 => vec![
            vec![-31, 8, 4, 0xfc],
            vec![-15, 9, 2, 0x1fc],
            vec![-11, 8, 2, 0xfd],
            vec![-7, 9, 1, 0x1fd],
            vec![-5, 7, 1, 0x7c],
            vec![-3, 4, 1, 0xa],
            vec![-1, 3, 1, 0x2],
            vec![1, 3, 1, 0x3],
            vec![3, 5, 1, 0x1a],
            vec![5, 6, 1, 0x3a],
            vec![7, 3, 5, 0x4],
            vec![39, 6, 2, 0x3b],
            vec![43, 4, 5, 0xb],
            vec![75, 4, 6, 0xc],
            vec![139, 5, 7, 0x1b],
            vec![267, 5, 8, 0x1c],
            vec![523, 6, 8, 0x3c],
            vec![779, 7, 9, 0x7d],
            vec![1291, 6, 11, 0x3d],
            vec![-32, 9, 32, 0x1fe, -1], // lower
            vec![3339, 9, 32, 0x1ff],    // upper
            vec![2, 0x0],                // OOB
        ],
        10 => vec![
            vec![-21, 7, 4, 0x7a],
            vec![-5, 8, 0, 0xfc],
            vec![-4, 7, 0, 0x7b],
            vec![-3, 5, 0, 0x18],
            vec![-2, 2, 2, 0x0],
            vec![2, 5, 0, 0x19],
            vec![3, 6, 0, 0x36],
            vec![4, 7, 0, 0x7c],
            vec![5, 8, 0, 0xfd],
            vec![6, 2, 6, 0x1],
            vec![70, 5, 5, 0x1a],
            vec![102, 6, 5, 0x37],
            vec![134, 6, 6, 0x38],
            vec![198, 6, 7, 0x39],
            vec![326, 6, 8, 0x3a],
            vec![582, 6, 9, 0x3b],
            vec![1094, 6, 10, 0x3c],
            vec![2118, 7, 11, 0x7d],
            vec![-22, 8, 32, 0xfe, -1], // lower
            vec![4166, 8, 32, 0xff],    // upper
            vec![2, 0x2],               // OOB
        ],
        11 => vec![
            vec![1, 1, 0, 0x0],
            vec![2, 2, 1, 0x2],
            vec![4, 4, 0, 0xc],
            vec![5, 4, 1, 0xd],
            vec![7, 5, 1, 0x1c],
            vec![9, 5, 2, 0x1d],
            vec![13, 6, 2, 0x3c],
            vec![17, 7, 2, 0x7a],
            vec![21, 7, 3, 0x7b],
            vec![29, 7, 4, 0x7c],
            vec![45, 7, 5, 0x7d],
            vec![77, 7, 6, 0x7e],
            vec![141, 7, 32, 0x7f], // upper
        ],
        12 => vec![
            vec![1, 1, 0, 0x0],
            vec![2, 2, 0, 0x2],
            vec![3, 3, 1, 0x6],
            vec![5, 5, 0, 0x1c],
            vec![6, 5, 1, 0x1d],
            vec![8, 6, 1, 0x3c],
            vec![10, 7, 0, 0x7a],
            vec![11, 7, 1, 0x7b],
            vec![13, 7, 2, 0x7c],
            vec![17, 7, 3, 0x7d],
            vec![25, 7, 4, 0x7e],
            vec![41, 8, 5, 0xfe],
            vec![73, 8, 32, 0xff], // upper
        ],
        13 => vec![
            vec![1, 1, 0, 0x0],
            vec![2, 3, 0, 0x4],
            vec![3, 4, 0, 0xc],
            vec![4, 5, 0, 0x1c],
            vec![5, 4, 1, 0xd],
            vec![7, 3, 3, 0x5],
            vec![15, 6, 1, 0x3a],
            vec![17, 6, 2, 0x3b],
            vec![21, 6, 3, 0x3c],
            vec![29, 6, 4, 0x3d],
            vec![45, 6, 5, 0x3e],
            vec![77, 7, 6, 0x7e],
            vec![141, 7, 32, 0x7f], // upper
        ],
        14 => vec![
            vec![-2, 3, 0, 0x4],
            vec![-1, 3, 0, 0x5],
            vec![0, 1, 0, 0x0],
            vec![1, 3, 0, 0x6],
            vec![2, 3, 0, 0x7],
        ],
        15 => vec![
            vec![-24, 7, 4, 0x7c],
            vec![-8, 6, 2, 0x3c],
            vec![-4, 5, 1, 0x1c],
            vec![-2, 4, 0, 0xc],
            vec![-1, 3, 0, 0x4],
            vec![0, 1, 0, 0x0],
            vec![1, 3, 0, 0x5],
            vec![2, 4, 0, 0xd],
            vec![3, 5, 1, 0x1d],
            vec![5, 6, 2, 0x3d],
            vec![9, 7, 4, 0x7d],
            vec![-25, 7, 32, 0x7e, -1], // lower
            vec![25, 7, 32, 0x7f],      // upper
        ],
        _ => unreachable!()
    };

    // Convert to HuffmanLine objects using unified constructor
    let mut lines = Vec::new();
    for line_data in lines_data {
        lines.push(HuffmanLine::new(&line_data));
    }

    let table = HuffmanTable::new(lines, true);
    
    table
}
