use crate::*;

#[test]
fn progression_lrcp() {
    check("packets_progression_lrcp", reference("main_rgba"));
}

#[test]
fn progression_rlcp() {
    check("packets_progression_rlcp", reference("main_rgba"));
}

#[test]
fn progression_rpcl() {
    check("packets_progression_rpcl", reference("main_rgba"));
}

#[test]
fn progression_pcrl() {
    check("packets_progression_pcrl", reference("main_rgba"));
}

#[test]
fn progression_cprl() {
    check("packets_progression_cprl", reference("main_rgba"));
}

#[test]
fn sop() {
    check("packets_sop", reference("main_r"));
}

#[test]
fn eph() {
    check("packets_eph", reference("main_r"));
}

#[test]
fn sop_eph() {
    check("packets_sop_eph", reference("main_r"));
}

#[test]
fn layers_many() {
    check("packets_layers_many", reference("main_r"));
}

#[test]
fn late_inclusion() {
    check("packets_late_inclusion", reference("main_r"));
}

#[test]
fn lblock_large() {
    check(
        "packets_lblock_large",
        reference("stress_16bit_crop_64_0_64_64"),
    );
}

#[test]
fn passes_codeword_long() {
    check(
        "packets_passes_codeword_long",
        reference("stress_16bit_crop_64_0_64_64"),
    );
}

#[test]
fn sop_missing() {
    check("packets_sop_missing", reference("main_r"));
}

#[test]
fn ppm() {
    check("packets_ppm", reference("main_rgb"));
}

#[test]
fn ppm_sop_eph() {
    check("packets_ppm_sop_eph", reference("main_rgb"));
}

#[test]
fn ppm_many_segments() {
    check("packets_ppm_many_segments", reference("main_rgb"));
}

#[test]
fn ppm_spanning_segments() {
    check("packets_ppm_spanning_segments", reference("main_r"));
}

#[test]
fn ppm_many_tiles() {
    check("packets_ppm_many_tiles", reference("main_r"));
}

#[test]
fn ppt() {
    check("packets_ppt", reference("main_rgb"));
}

#[test]
fn ppt_many_segments() {
    check("packets_ppt_many_segments", reference("main_rgb"));
}

#[test]
fn ppt_sop_eph_tiny_tiles() {
    check(
        "packets_ppt_sop_eph_tiny_tiles",
        reference("main_rgb_crop_50_40_12_12"),
    );
}

#[test]
fn poc_lrcp() {
    check("packets_poc_lrcp", reference("main_rgb"));
}

#[test]
fn poc_rlcp() {
    check("packets_poc_rlcp", reference("main_rgb"));
}

#[test]
fn poc_rpcl() {
    check("packets_poc_rpcl", reference("main_rgb"));
}

#[test]
fn poc_pcrl() {
    check("packets_poc_pcrl", reference("main_rgb"));
}

#[test]
fn poc_cprl() {
    check("packets_poc_cprl", reference("main_rgb"));
}

#[test]
fn poc_main_header() {
    check("packets_poc_main_header", reference("main_rgb"));
}

#[test]
fn poc_main_header_tile_override() {
    check(
        "packets_poc_main_header_tile_override",
        reference("main_rgb"),
    );
}

#[test]
fn poc_per_tile() {
    check("packets_poc_per_tile", reference("main_rgb"));
}

#[test]
fn poc_main_header_ranges() {
    check("packets_poc_main_header_ranges", reference("main_rgb"));
}

#[test]
fn poc_clamped_values() {
    check("packets_poc_clamped_values", reference("main_rgb"));
}

#[test]
fn poc_subsampled() {
    check(
        "packets_poc_subsampled",
        reference("main_rgb_sub_1x1_2x2_1x2"),
    );
}

#[test]
fn poc_offset() {
    check("packets_poc_offset", reference("main_rgb"));
}

#[test]
fn poc_tile_parts() {
    check("packets_poc_tile_parts", reference("main_rgb"));
}

#[test]
fn mixed_levels_lrcp() {
    check("packets_mixed_levels_lrcp", reference("main_rgb"));
}

#[test]
fn mixed_levels_rlcp() {
    check("packets_mixed_levels_rlcp", reference("main_rgb"));
}

#[test]
fn mixed_levels_rpcl() {
    check("packets_mixed_levels_rpcl", reference("main_rgb"));
}

#[test]
fn mixed_levels_pcrl() {
    check("packets_mixed_levels_pcrl", reference("main_rgb"));
}

#[test]
fn mixed_levels_cprl() {
    check("packets_mixed_levels_cprl", reference("main_rgb"));
}

#[test]
fn empty_packets() {
    check("packets_empty_packets", reference("main_rgba"));
}

#[test]
fn eph_empty_packets() {
    check("packets_eph_empty_packets", reference("main_rgba"));
}

#[test]
fn invalid_stuff_bit() {
    check(
        "packets_invalid_stuff_bit",
        reference("packets_invalid_stuff_bit"),
    );
}

fn many_components(count: u16, poc_start: Option<u16>) -> Vec<Vec<u8>> {
    let tiny = include_bytes!("streams/codestream_tiny.j2k");
    let position = |marker| {
        tiny.windows(2)
            .position(|bytes| bytes == [0xff, marker])
            .unwrap()
    };
    let cod = position(0x52);
    let sot = position(0x90);
    let body = &tiny[position(0x93) + 2..position(0xd9)];
    let mut stream = tiny[..cod - 5].to_vec();
    stream[4..6].copy_from_slice(&(38 + 3 * count).to_be_bytes());
    stream.extend(count.to_be_bytes());
    for _ in 0..count {
        stream.extend_from_slice(&tiny[cod - 3..cod]);
    }
    let new_cod = stream.len();
    stream.extend_from_slice(&tiny[cod..sot]);
    stream[new_cod + 5] = 4;
    if let Some(start) = poc_start {
        stream.extend([0xff, 0x5f, 0, 11, 0]);
        stream.extend(start.to_be_bytes());
        stream.extend([0, 1, 4]);
        stream.extend(count.to_be_bytes());
        stream.push(4);
    }
    let packets = body.repeat(usize::from(count - poc_start.unwrap_or(0)));
    stream.extend_from_slice(&tiny[sot..sot + 6]);
    stream.extend(u32::try_from(14 + packets.len()).unwrap().to_be_bytes());
    stream.extend([0, 1, 0xff, 0x93]);
    stream.extend(packets);
    stream.extend([0xff, 0xd9]);

    let settings = DecodeSettings {
        strict: true,
        ..DecodeSettings::default()
    };
    let image = Image::from_codestream(&stream, 1, &settings).unwrap();
    image
        .decode(&mut DecoderContext::default())
        .unwrap()
        .components()
        .iter()
        .map(|component| component.samples().iter().map(|&v| v as u8).collect())
        .collect()
}

#[test]
fn default_order_visits_all_256_components() {
    let expected = many_components(1, None).pop().unwrap();
    assert!(expected.windows(2).any(|pair| pair[0] != pair[1]));
    assert_eq!(many_components(256, None), vec![expected; 256]);
}

#[test]
fn poc_can_start_at_component_256() {
    let expected = many_components(1, None).pop().unwrap();
    let components = many_components(257, Some(256));
    assert_eq!(components[256], expected);
    assert!(
        components[..256]
            .iter()
            .all(|component| component.iter().all(|&value| value == 128))
    );
}
