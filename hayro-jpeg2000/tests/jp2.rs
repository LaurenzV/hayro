//! The JP2 and JPX file formats (Annex I, T.801 Annex M).

use crate::*;

#[test]
fn colr_srgb() {
    check("jp2_colr_srgb", reference("main_rgb"));
}

#[test]
fn colr_greyscale() {
    check("jp2_colr_greyscale", reference("main_r"));
}

#[test]
fn colr_esrgb() {
    check("jp2_colr_esrgb", reference("main_rgb"));
}

#[test]
fn gray_alpha() {
    check("jp2_gray_alpha", reference("main_ra"));
}

#[test]
fn alpha_whole_image() {
    check("jp2_alpha_whole_image", reference("main_ra"));
}

#[test]
fn rgba() {
    check("jp2_rgba", reference("main_rgba"));
}

#[test]
fn cdef_alpha_not_last() {
    check("jp2_cdef_alpha_not_last", reference("main_rgba"));
}

#[test]
fn cdef_reordered() {
    check("jp2_cdef_reordered", reference("main_rgba"));
}

#[test]
fn bpcc() {
    check("jp2_bpcc", reference("main_rgba_5_5_5_1bit"));
}

#[test]
fn jpx_brand_rreq() {
    check("jp2_jpx_brand_rreq", reference("main_rgb"));
}

#[test]
fn extra_boxes() {
    check("jp2_extra_boxes", reference("main_rgb"));
}

#[test]
fn xl_box() {
    check("jp2_xl_box", reference("main_rgb"));
}

#[test]
fn jp2c_length_zero() {
    check("jp2_jp2c_length_zero", reference("main_rgb"));
}

#[test]
fn box_longer_than_file() {
    check("jp2_box_longer_than_file", reference("main_rgb"));
}

#[test]
fn trailing_box_longer_than_file() {
    check("jp2_trailing_box_longer_than_file", reference("main_rgb"));
}

#[test]
fn trailing_zero_box() {
    check("jp2_trailing_zero_box", reference("main_rgb"));
}

#[test]
fn duplicate_ihdr() {
    check("jp2_duplicate_ihdr", reference("main_rgb"));
}

#[test]
fn colr_sycc_444() {
    check("jp2_colr_sycc_444", reference("jp2_colr_sycc_444"));
}

#[test]
fn colr_sycc_422() {
    check("jp2_colr_sycc_422", reference("jp2_colr_sycc_422"));
}

#[test]
fn colr_sycc_420() {
    check("jp2_colr_sycc_420", reference("jp2_colr_sycc_420"));
}

#[test]
fn cdef_reversed_sycc() {
    check(
        "jp2_cdef_reversed_sycc",
        reference("jp2_cdef_reversed_sycc"),
    );
}

#[test]
fn colr_first_wins() {
    check("jp2_colr_first_wins", reference("jp2_cdef_reversed_sycc"));
}

#[test]
fn colr_ycck() {
    check("jp2_colr_ycck", reference("jp2_colr_ycck"));
}
