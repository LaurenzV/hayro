pub(crate) mod sb_operator {
    pub(crate) const HORIZONTAL_STEM: u8 = 1;
    pub(crate) const VERTICAL_STEM: u8 = 3;
    pub(crate) const VERTICAL_MOVE_TO: u8 = 4;
    pub(crate) const LINE_TO: u8 = 5;
    pub(crate) const HORIZONTAL_LINE_TO: u8 = 6;
    pub(crate) const VERTICAL_LINE_TO: u8 = 7;
    pub(crate) const CURVE_TO: u8 = 8;
    pub(crate) const CLOSE_PATH: u8 = 9;
    pub(crate) const CALL_SUBR: u8 = 10;
    pub(crate) const RETURN: u8 = 11;
    pub(crate) const ESCAPE: u8 = 12;
    pub(crate) const HSBW: u8 = 13;
    pub(crate) const ENDCHAR: u8 = 14;
    pub(crate) const MOVE_TO: u8 = 21;
    pub(crate) const HORIZONTAL_MOVE_TO: u8 = 22;
    pub(crate) const VH_CURVE_TO: u8 = 30;
    pub(crate) const HV_CURVE_TO: u8 = 31;
}

pub(crate) mod tb_operator {
    pub(crate) const DOTSECTION: u8 = 0;
    pub(crate) const VSTEM3: u8 = 1;
    pub(crate) const HSTEM3: u8 = 2;
    pub(crate) const SEAC: u8 = 6;
    pub(crate) const SBW: u8 = 7;
    pub(crate) const DIV: u8 = 12;
    pub(crate) const CALL_OTHER_SUBR: u8 = 16;
    pub(crate) const POP: u8 = 17;
    pub(crate) const SET_CURRENT_POINT: u8 = 33;
}
