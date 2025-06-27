use crate::OutlineError;
use crate::argstack::ArgumentsStack;
use crate::type1::charstring_parser::CharStringParser;
use crate::type1::operator::{sb_operator, tb_operator};
use crate::type1::stream::Stream;
use crate::type1::{EncodingType, Parameters};
use crate::{Builder, OutlineBuilder, RectF};
use log::{debug, error, trace, warn};

const MAX_ARGUMENTS_STACK_LEN: usize = 48;
const STACK_LIMIT: u8 = 10;

struct CharStringParserContext<'a> {
    params: &'a Parameters,
    stems_len: u32,
    has_endchar: bool,
    has_seac: bool,
}

pub(crate) fn parse_char_string(
    data: &[u8],
    params: &Parameters,
    builder: &mut dyn OutlineBuilder,
) -> Result<(), OutlineError> {
    let mut ctx = CharStringParserContext {
        params,
        stems_len: 0,
        has_endchar: false,
        has_seac: false,
    };

    let mut inner_builder = Builder {
        builder,
        bbox: RectF::new(),
    };

    let stack = ArgumentsStack {
        data: &mut [0.0; MAX_ARGUMENTS_STACK_LEN],
        len: 0,
        max_len: MAX_ARGUMENTS_STACK_LEN,
    };

    let mut parser = CharStringParser {
        stack,
        builder: &mut inner_builder,
        x: 0.0,
        y: 0.0,
        sbx: 0.0,
        is_flexing: false,
    };
    _parse_char_string(&mut ctx, data, 0, &mut parser)?;

    if !ctx.has_endchar {
        return Err(OutlineError::MissingEndChar);
    }

    Ok(())
}

fn _parse_char_string(
    ctx: &mut CharStringParserContext,
    char_string: &[u8],
    depth: u8,
    p: &mut CharStringParser,
) -> Result<(), OutlineError> {
    macro_rules! trace_op {
        ($name:literal) => {
            debug!("{} ({})", $name, &p.stack.dump());
        };
    }

    let mut s = Stream::new(char_string);
    while !s.at_end() {
        let op = s.read_byte().ok_or(OutlineError::ReadOutOfBounds)?;
        match op {
            sb_operator::HORIZONTAL_STEM | sb_operator::VERTICAL_STEM => {
                trace_op!("HORIZONTAL_STEM | VERTICAL_STEM");
                let len = p.stack.len();

                ctx.stems_len += len as u32 >> 1;

                p.stack.clear();
            }
            sb_operator::VERTICAL_MOVE_TO => {
                trace_op!("VERTICAL_MOVE_TO");

                p.parse_vertical_move_to()?;
            }
            sb_operator::LINE_TO => {
                trace_op!("LINE_TO");

                p.parse_line_to()?;
            }
            sb_operator::HORIZONTAL_LINE_TO => {
                trace_op!("HORIZONTAL_LINE_TO");

                p.parse_horizontal_line_to()?;
            }
            sb_operator::VERTICAL_LINE_TO => {
                trace_op!("VERTICAL_LINE_TO");

                p.parse_vertical_line_to()?;
            }
            sb_operator::CURVE_TO => {
                trace_op!("CURVE_TO");

                p.parse_curve_to()?;
            }
            sb_operator::CLOSE_PATH => {
                trace_op!("CLOSE_PATH");

                p.parse_close_path()?;
            }
            sb_operator::CALL_SUBR => {
                trace_op!("CALL_SUBR");

                if p.stack.is_empty() {
                    return Err(OutlineError::InvalidArgumentsStackLength);
                }

                if depth == STACK_LIMIT {
                    return Err(OutlineError::NestingLimitReached);
                }

                let index = p.stack.pop() as u32;

                if let Some(subr) = ctx.params.subroutines.get(&index) {
                    _parse_char_string(ctx, subr, depth + 1, p)?;
                } else {
                    return Err(OutlineError::NoLocalSubroutines);
                }
            }
            sb_operator::RETURN => {
                trace_op!("RETURN");

                break;
            }
            sb_operator::ESCAPE => {
                let op = s.read_byte().ok_or(OutlineError::ReadOutOfBounds)?;

                match op {
                    tb_operator::DOTSECTION => {
                        trace_op!("DOTSECTION");

                        p.stack.clear();
                    }
                    tb_operator::VSTEM3 => {
                        trace_op!("VSTEM3");

                        p.stack.clear();
                    }
                    tb_operator::HSTEM3 => {
                        trace_op!("HSTEM3");

                        p.stack.clear();
                    }
                    tb_operator::SEAC => {
                        trace_op!("SEAC");

                        if p.stack.len != 5 {
                            return Err(OutlineError::InvalidArgumentsStackLength);
                        }

                        let accent_char = EncodingType::Standard.encode(p.stack.pop() as u8);
                        let base_char = EncodingType::Standard.encode(p.stack.pop() as u8);
                        let dy = p.stack.pop();
                        let dx = p.stack.pop();
                        let asb = p.stack.pop();

                        ctx.has_seac = true;

                        if depth == STACK_LIMIT {
                            return Err(OutlineError::NestingLimitReached);
                        }

                        let base_char_string = ctx
                            .params
                            .charstrings
                            .get(&base_char.ok_or(OutlineError::InvalidSeacCode)?.to_string())
                            .ok_or(OutlineError::InvalidSeacCode)?;
                        _parse_char_string(ctx, base_char_string, depth + 1, p)?;
                        p.x = dx + p.sbx - asb;
                        p.y = dy;

                        let accent_char_string = ctx
                            .params
                            .charstrings
                            .get(
                                &accent_char
                                    .ok_or(OutlineError::InvalidSeacCode)?
                                    .to_string(),
                            )
                            .ok_or(OutlineError::InvalidSeacCode)?;
                        _parse_char_string(ctx, accent_char_string, depth + 1, p)?;
                        break;
                    }
                    tb_operator::SBW => {
                        trace_op!("SBW");
                        p.x = p.stack.at(0);
                        p.y = p.stack.at(1);
                        p.sbx = p.x;

                        p.stack.clear();
                    }
                    tb_operator::DIV => {
                        trace_op!("DIV");
                        let num2 = p.stack.pop();
                        let num1 = p.stack.pop();

                        p.stack.push(num1 / num2)?;
                    }
                    tb_operator::CALL_OTHER_SUBR => {
                        trace_op!("CALL_OTHER_SUBR");

                        let subr_index = p.stack.pop() as i32;
                        let n_args = p.stack.pop() as i32;

                        if subr_index == 1 && n_args == 0 {
                            p.is_flexing = true;
                        } else if subr_index == 0 && n_args == 3 {
                            p.parse_flex()?;
                            p.is_flexing = false;
                        } else {
                            trace!("ignoring call_other_subr with {}, {}", subr_index, n_args);
                        }
                    }
                    tb_operator::POP => {
                        trace_op!("POP");
                    }
                    tb_operator::SET_CURRENT_POINT => {
                        trace_op!("SET_CURRENT_POINT");
                        p.x = p.stack.at(0);
                        p.y = p.stack.at(1);

                        p.stack.clear();
                    }
                    _ => error!("unknown two-byte operator {op}"),
                }
            }
            sb_operator::HSBW => {
                trace_op!("HSBW");

                p.x += p.stack.at(0);
                p.sbx = p.x;

                p.stack.clear();
            }
            sb_operator::ENDCHAR => {
                trace_op!("ENDCHAR");
                ctx.has_endchar = true;

                break;
            }
            sb_operator::MOVE_TO => {
                trace_op!("MOVE_TO");

                p.parse_move_to()?;
            }
            sb_operator::HORIZONTAL_MOVE_TO => {
                trace_op!("HORIZONTAL_MOVE_TO");

                p.parse_horizontal_move_to()?;
            }
            sb_operator::VH_CURVE_TO => {
                trace_op!("VH_CURVE_TO");

                p.parse_vh_curve_to()?;
            }
            sb_operator::HV_CURVE_TO => {
                trace_op!("HV_CURVE_TO");

                p.parse_hv_curve_to()?;
            }
            32..=246 => {
                p.parse_int1(op)?;
            }
            247..=250 => {
                p.parse_int2(op, &mut s)?;
            }
            251..=254 => {
                p.parse_int3(op, &mut s)?;
            }
            255 => p.parse_int4(&mut s)?,
            _ => {
                warn!("unrecognized charstring op: {}", op);
            }
        }
    }

    Ok(())
}
