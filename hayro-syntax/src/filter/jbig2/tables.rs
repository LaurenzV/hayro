// Constants for segment types (7.3 Segment types) - matches JS SegmentTypes array exactly
pub(crate) const SEGMENT_TYPES: &[Option<&str>] = &[
    Some("SymbolDictionary"),
    None,
    None,
    None,
    Some("IntermediateTextRegion"),
    None,
    Some("ImmediateTextRegion"),
    Some("ImmediateLosslessTextRegion"),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some("PatternDictionary"),
    None,
    None,
    None,
    Some("IntermediateHalftoneRegion"),
    None,
    Some("ImmediateHalftoneRegion"),
    Some("ImmediateLosslessHalftoneRegion"),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some("IntermediateGenericRegion"),
    None,
    Some("ImmediateGenericRegion"),
    Some("ImmediateLosslessGenericRegion"),
    Some("IntermediateGenericRefinementRegion"),
    None,
    Some("ImmediateGenericRefinementRegion"),
    Some("ImmediateLosslessGenericRefinementRegion"),
    None,
    None,
    None,
    None,
    Some("PageInformation"),
    Some("EndOfPage"),
    Some("EndOfStripe"),
    Some("EndOfFile"),
    Some("Profiles"),
    Some("Tables"),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    Some("Extension"),
];

// Coding templates
pub(crate) const CODING_TEMPLATES: [&[[i32; 2]]; 4] = [
    &[
        [-1, -2],
        [0, -2],
        [1, -2],
        [-2, -1],
        [-1, -1],
        [0, -1],
        [1, -1],
        [2, -1],
        [-4, 0],
        [-3, 0],
        [-2, 0],
        [-1, 0],
    ],
    &[
        [-1, -2],
        [0, -2],
        [1, -2],
        [2, -2],
        [-2, -1],
        [-1, -1],
        [0, -1],
        [1, -1],
        [2, -1],
        [-3, 0],
        [-2, 0],
        [-1, 0],
    ],
    &[
        [-1, -2],
        [0, -2],
        [1, -2],
        [-2, -1],
        [-1, -1],
        [0, -1],
        [1, -1],
        [-2, 0],
        [-1, 0],
    ],
    &[
        [-3, -1],
        [-2, -1],
        [-1, -1],
        [0, -1],
        [1, -1],
        [-4, 0],
        [-3, 0],
        [-2, 0],
        [-1, 0],
    ],
];

// Refinement templates
pub(crate) const REFINEMENT_TEMPLATES: [RefinementTemplate; 2] = [
    RefinementTemplate {
        coding: &[[0, -1], [1, -1], [-1, 0]],
        reference: &[
            [0, -1],
            [1, -1],
            [-1, 0],
            [0, 0],
            [1, 0],
            [-1, 1],
            [0, 1],
            [1, 1],
        ],
    },
    RefinementTemplate {
        coding: &[[-1, -1], [0, -1], [1, -1], [-1, 0]],
        reference: &[[0, -1], [-1, 0], [0, 0], [1, 0], [0, 1], [1, 1]],
    },
];

pub(crate) struct RefinementTemplate {
    pub(crate) coding: &'static [[i32; 2]],
    pub(crate) reference: &'static [[i32; 2]],
}

// Reused contexts for different template indices (6.2.5.7)
pub(crate) const REUSED_CONTEXTS: [u32; 4] = [
    0x9b25, // 10011 0110010 0101
    0x0795, // 0011 110010 101
    0x00e5, // 001 11001 01
    0x0195, // 011001 0101
];

// Refinement reused contexts
pub(crate) const REFINEMENT_REUSED_CONTEXTS: [u32; 2] = [
    0x0020, // '000' + '0' (coding) + '00010000' + '0' (reference)
    0x0008, // '0000' + '001000'
];

// QM Coder Table C-2 from JPEG 2000 Part I Final Committee Draft Version 1.0
#[derive(Clone, Copy)]
pub(crate) struct QeEntry {
    pub(crate) qe: u32,
    pub(crate) nmps: u8,
    pub(crate) nlps: u8,
    pub(crate) switch_flag: u8,
}

pub(crate) const QE_TABLE: [QeEntry; 47] = [
    QeEntry {
        qe: 0x5601,
        nmps: 1,
        nlps: 1,
        switch_flag: 1,
    },
    QeEntry {
        qe: 0x3401,
        nmps: 2,
        nlps: 6,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1801,
        nmps: 3,
        nlps: 9,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0ac1,
        nmps: 4,
        nlps: 12,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0521,
        nmps: 5,
        nlps: 29,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0221,
        nmps: 38,
        nlps: 33,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x5601,
        nmps: 7,
        nlps: 6,
        switch_flag: 1,
    },
    QeEntry {
        qe: 0x5401,
        nmps: 8,
        nlps: 14,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x4801,
        nmps: 9,
        nlps: 14,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x3801,
        nmps: 10,
        nlps: 14,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x3001,
        nmps: 11,
        nlps: 17,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x2401,
        nmps: 12,
        nlps: 18,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1c01,
        nmps: 13,
        nlps: 20,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1601,
        nmps: 29,
        nlps: 21,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x5601,
        nmps: 15,
        nlps: 14,
        switch_flag: 1,
    },
    QeEntry {
        qe: 0x5401,
        nmps: 16,
        nlps: 14,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x5101,
        nmps: 17,
        nlps: 15,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x4801,
        nmps: 18,
        nlps: 16,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x3801,
        nmps: 19,
        nlps: 17,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x3401,
        nmps: 20,
        nlps: 18,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x3001,
        nmps: 21,
        nlps: 19,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x2801,
        nmps: 22,
        nlps: 19,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x2401,
        nmps: 23,
        nlps: 20,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x2201,
        nmps: 24,
        nlps: 21,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1c01,
        nmps: 25,
        nlps: 22,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1801,
        nmps: 26,
        nlps: 23,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1601,
        nmps: 27,
        nlps: 24,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1401,
        nmps: 28,
        nlps: 25,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1201,
        nmps: 29,
        nlps: 26,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x1101,
        nmps: 30,
        nlps: 27,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0ac1,
        nmps: 31,
        nlps: 28,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x09c1,
        nmps: 32,
        nlps: 29,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x08a1,
        nmps: 33,
        nlps: 30,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0521,
        nmps: 34,
        nlps: 31,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0441,
        nmps: 35,
        nlps: 32,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x02a1,
        nmps: 36,
        nlps: 33,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0221,
        nmps: 37,
        nlps: 34,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0141,
        nmps: 38,
        nlps: 35,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0111,
        nmps: 39,
        nlps: 36,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0085,
        nmps: 40,
        nlps: 37,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0049,
        nmps: 41,
        nlps: 38,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0025,
        nmps: 42,
        nlps: 39,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0015,
        nmps: 43,
        nlps: 40,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0009,
        nmps: 44,
        nlps: 41,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0005,
        nmps: 45,
        nlps: 42,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x0001,
        nmps: 45,
        nlps: 43,
        switch_flag: 0,
    },
    QeEntry {
        qe: 0x5601,
        nmps: 46,
        nlps: 46,
        switch_flag: 0,
    },
];
