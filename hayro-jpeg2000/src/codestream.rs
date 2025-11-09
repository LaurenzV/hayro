use crate::bitmap::ChannelData;
use crate::decode::{SubBandType, decode};
use hayro_common::byte::Reader;

pub(crate) fn read(stream: &[u8]) -> Result<(Header, Vec<ChannelData>), &'static str> {
    let mut reader = Reader::new(stream);

    let marker = reader.read_marker()?;
    if marker != markers::SOC {
        return Err("invalid marker: expected SOC marker");
    }

    let header = read_header(&mut reader)?;
    let code_stream_data = reader
        .tail()
        .ok_or("code stream data is missing from image")?;
    let decoded = decode(code_stream_data, &header)?;

    Ok((header, decoded))
}

#[derive(Debug)]
pub(crate) struct Header {
    pub(crate) size_data: SizeData,
    pub(crate) global_coding_style: CodingStyleDefault,
    pub(crate) component_infos: Vec<ComponentInfo>,
}

fn read_header(reader: &mut Reader) -> Result<Header, &'static str> {
    if reader.read_marker()? != markers::SIZ {
        return Err("expected SIZ marker after SOC");
    }

    let size_data = size_marker(reader)?;

    let mut cod = None;
    let mut qcd = None;

    let num_components = size_data.component_sizes.len() as u16;
    let mut cod_components = vec![None; num_components as usize];
    let mut qcd_components = vec![None; num_components as usize];

    loop {
        match reader.peek_marker().ok_or("failed to read marker")? {
            markers::SOT => break,
            markers::COD => {
                reader.read_marker()?;
                cod = Some(cod_marker(reader).ok_or("failed to read COD marker")?);
            }
            markers::COC => {
                reader.read_marker()?;
                let (component_index, coc) =
                    coc_marker(reader, num_components).ok_or("failed to read COC marker")?;
                cod_components[component_index as usize] = Some(coc);
            }
            markers::QCD => {
                reader.read_marker()?;
                qcd = Some(qcd_marker(reader).ok_or("failed to read QCD marker")?);
            }
            markers::QCC => {
                reader.read_marker()?;
                let (component_index, qcc) =
                    qcc_marker(reader, num_components).ok_or("failed to read QCC marker")?;
                qcd_components[component_index as usize] = Some(qcc);
            }
            markers::RGN => {
                reader.read_marker()?;
                rgn_marker(reader).ok_or("failed to read RGN marker")?;
            }
            markers::TLM => {
                reader.read_marker()?;
                tlm_marker(reader).ok_or("failed to read TLM marker")?;
            }
            markers::COM => {
                reader.read_marker()?;
                com_marker(reader).ok_or("failed to read COM marker")?;
            }
            _ => {
                return Err("unsupported marker encountered in main header");
            }
        }
    }

    let cod = cod.ok_or("missing COD marker")?;
    let qcd = qcd.ok_or("missing QCD marker")?;

    let component_infos: Vec<ComponentInfo> = size_data
        .component_sizes
        .iter()
        .enumerate()
        .map(|(idx, csi)| ComponentInfo {
            size_info: *csi,
            coding_style: cod_components[idx]
                .clone()
                .unwrap_or(cod.component_parameters.clone()),
            quantization_info: qcd_components[idx].clone().unwrap_or(qcd.clone()),
        })
        .collect();

    for ci in &component_infos {
        if ci
            .coding_style
            .parameters
            .code_block_style
            .selective_arithmetic_coding_bypass
        {
            return Err("unsupported code-block style features encountered during decoding");
        }
    }

    Ok(Header {
        size_data,
        global_coding_style: cod.clone(),
        component_infos,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct ComponentInfo {
    pub(crate) size_info: ComponentSizeInfo,
    pub(crate) coding_style: CodingStyleComponent,
    pub(crate) quantization_info: QuantizationInfo,
}

impl ComponentInfo {
    pub(crate) fn exponent_mantissa(
        &self,
        sub_band_type: SubBandType,
        resolution: u16,
    ) -> (u16, u16) {
        let n_ll = self.coding_style.parameters.num_decomposition_levels;

        let sb_index = match sub_band_type {
            // TODO: Shouldn't be reached.
            SubBandType::LowLow => u16::MAX,
            SubBandType::HighLow => 0,
            SubBandType::LowHigh => 1,
            SubBandType::HighHigh => 2,
        };

        let step_sizes = &self.quantization_info.step_sizes;
        match self.quantization_info.quantization_style {
            QuantizationStyle::NoQuantization | QuantizationStyle::ScalarExpounded => {
                let entry = if resolution == 0 {
                    step_sizes[0]
                } else {
                    step_sizes[(1 + (resolution - 1) * 3 + sb_index) as usize]
                };

                (entry.exponent, entry.mantissa)
            }
            QuantizationStyle::ScalarDerived => {
                let e_0 = step_sizes[0].exponent;
                let mantissa = step_sizes[0].mantissa;
                let n_b = if resolution == 0 {
                    n_ll
                } else {
                    n_ll + 1 - resolution
                };

                (e_0 - n_ll + n_b, mantissa)
            }
        }
    }

    pub(crate) fn wavelet_transform(&self) -> WaveletTransform {
        self.coding_style.parameters.transformation
    }

    pub(crate) fn num_resolution_levels(&self) -> u16 {
        self.coding_style.parameters.num_resolution_levels
    }
}

/// Progression order (Table A.16).
#[derive(Debug, Clone, Copy)]
pub(crate) enum ProgressionOrder {
    LayerResolutionComponentPosition,
    ResolutionLayerComponentPosition,
    ResolutionPositionComponentLayer,
    PositionComponentResolutionLayer,
    ComponentPositionResolutionLayer,
}

impl ProgressionOrder {
    fn from_u8(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(ProgressionOrder::LayerResolutionComponentPosition),
            1 => Ok(ProgressionOrder::ResolutionLayerComponentPosition),
            2 => Ok(ProgressionOrder::ResolutionPositionComponentLayer),
            3 => Ok(ProgressionOrder::PositionComponentResolutionLayer),
            4 => Ok(ProgressionOrder::ComponentPositionResolutionLayer),
            _ => Err("invalid progression order"),
        }
    }
}

/// Wavelet transformation type (Table A.20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaveletTransform {
    Irreversible97,
    Reversible53,
}

impl WaveletTransform {
    fn from_u8(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(WaveletTransform::Irreversible97),
            1 => Ok(WaveletTransform::Reversible53),
            _ => Err("invalid transformation type"),
        }
    }
}

/// Coding style flags (Table A.13).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CodingStyleFlags {
    raw: u8,
}

impl CodingStyleFlags {
    fn from_u8(value: u8) -> Self {
        CodingStyleFlags { raw: value }
    }

    pub(crate) fn has_precincts(&self) -> bool {
        (self.raw & 0x01) != 0
    }

    pub(crate) fn may_use_sop_markers(&self) -> bool {
        (self.raw & 0x02) != 0
    }

    pub(crate) fn uses_eph_marker(&self) -> bool {
        (self.raw & 0x04) != 0
    }
}

/// Code-block style flags (Table A.19).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CodeBlockStyle {
    pub(crate) selective_arithmetic_coding_bypass: bool,
    pub(crate) reset_context_probabilities: bool,
    pub(crate) termination_on_each_pass: bool,
    pub(crate) vertically_causal_context: bool,
    pub(crate) _predictable_termination: bool,
    pub(crate) segmentation_symbols: bool,
}

impl CodeBlockStyle {
    fn from_u8(value: u8) -> Self {
        CodeBlockStyle {
            selective_arithmetic_coding_bypass: (value & 0x01) != 0,
            reset_context_probabilities: (value & 0x02) != 0,
            termination_on_each_pass: (value & 0x04) != 0,
            vertically_causal_context: (value & 0x08) != 0,
            _predictable_termination: (value & 0x10) != 0,
            segmentation_symbols: (value & 0x20) != 0,
        }
    }
}

/// Quantization style (Table A.28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuantizationStyle {
    NoQuantization,
    ScalarDerived,
    ScalarExpounded,
}

impl QuantizationStyle {
    fn from_u8(value: u8) -> Result<Self, &'static str> {
        match value & 0x1F {
            0 => Ok(QuantizationStyle::NoQuantization),
            1 => Ok(QuantizationStyle::ScalarDerived),
            2 => Ok(QuantizationStyle::ScalarExpounded),
            _ => Err("invalid quantization style"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StepSize {
    pub(crate) mantissa: u16,
    pub(crate) exponent: u16,
}

/// Quantization properties, from the QCD and QCC markers (A.6.4 and A.6.5).
#[derive(Clone, Debug)]
pub(crate) struct QuantizationInfo {
    pub(crate) quantization_style: QuantizationStyle,
    pub(crate) guard_bits: u8,
    pub(crate) step_sizes: Vec<StepSize>,
}

/// Default values for coding style, from the COD marker (A.6.1).
#[derive(Debug, Clone)]
pub(crate) struct CodingStyleDefault {
    pub(crate) progression_order: ProgressionOrder,
    pub(crate) num_layers: u16,
    pub(crate) mct: bool,
    // This is the default used for all components, if not overridden by COC.
    pub(crate) component_parameters: CodingStyleComponent,
}

/// Values of coding style for each component, from the COC marker (A.6.2).
#[derive(Clone, Debug)]
pub(crate) struct CodingStyleComponent {
    pub(crate) flags: CodingStyleFlags,
    pub(crate) parameters: CodingStyleParameters,
}

/// Shared parameters between the COC and COD marker (A.6.1 and A.6.2).
#[derive(Clone, Debug)]
pub(crate) struct CodingStyleParameters {
    pub(crate) num_decomposition_levels: u16,
    pub(crate) num_resolution_levels: u16,
    pub(crate) code_block_width: u8,
    pub(crate) code_block_height: u8,
    pub(crate) code_block_style: CodeBlockStyle,
    pub(crate) transformation: WaveletTransform,
    pub(crate) precinct_exponents: Vec<(u8, u8)>,
}

#[derive(Debug)]
pub(crate) struct SizeData {
    /// Width of the reference grid (Xsiz).
    pub(crate) reference_grid_width: u32,
    /// Height of the reference grid (Ysiz).
    pub(crate) reference_grid_height: u32,
    /// Horizontal offset from the origin of the reference grid to the
    /// left side of the image area (XOsiz).
    pub(crate) image_area_x_offset: u32,
    /// Vertical offset from the origin of the reference grid to the top side of the image area (YOsiz).
    pub(crate) image_area_y_offset: u32,
    /// Width of one reference tile with respect to the reference grid (XTSiz).
    pub(crate) tile_width: u32,
    /// Height of one reference tile with respect to the reference grid (YTSiz).
    pub(crate) tile_height: u32,
    /// Horizontal offset from the origin of the reference grid to the left side of the first tile (XTOSiz).
    pub(crate) tile_x_offset: u32,
    /// Vertical offset from the origin of the reference grid to the top side of the first tile (YTOSiz).
    pub(crate) tile_y_offset: u32,
    /// Component information (SSiz/XRSiz/YRSiz).
    pub(crate) component_sizes: Vec<ComponentSizeInfo>,
}

impl SizeData {
    pub(crate) fn tile_x_coord(&self, idx: u32) -> u32 {
        // See B-6.
        idx % self.num_x_tiles()
    }

    pub(crate) fn tile_y_coord(&self, idx: u32) -> u32 {
        // See B-6.
        (idx as f64 / self.num_x_tiles() as f64).floor() as u32
    }
}

/// Component information (A.5.1 and Table A.11).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ComponentSizeInfo {
    pub(crate) precision: u8,
    // TODO: What is this field for?
    pub(crate) _is_signed: bool,
    pub(crate) horizontal_resolution: u8,
    pub(crate) vertical_resolution: u8,
}

impl SizeData {
    /// The number of tiles in the x direction.
    pub(crate) fn num_x_tiles(&self) -> u32 {
        // See formula B-5.
        (self.reference_grid_width - self.tile_x_offset).div_ceil(self.tile_width)
    }

    /// The number of tiles in the y direction.
    pub(crate) fn num_y_tiles(&self) -> u32 {
        // See formula B-5.
        (self.reference_grid_height - self.tile_y_offset).div_ceil(self.tile_height)
    }

    /// The total number of tiles.
    pub(crate) fn num_tiles(&self) -> u32 {
        self.num_x_tiles() * self.num_y_tiles()
    }

    /// Return the overall width of the image.
    pub(crate) fn image_width(&self) -> u32 {
        self.reference_grid_width - self.image_area_x_offset
    }

    /// Return the overall height of the image.
    pub(crate) fn image_height(&self) -> u32 {
        self.reference_grid_height - self.image_area_y_offset
    }
}

/// SIZ marker (A.5.1).
fn size_marker(reader: &mut Reader) -> Result<SizeData, &'static str> {
    let size_data = size_marker_inner(reader).ok_or("failed to read SIZ marker")?;

    if size_data.tile_width == 0
        || size_data.tile_height == 0
        || size_data.reference_grid_width == 0
        || size_data.reference_grid_height == 0
    {
        return Err("invalid image dimensions");
    }

    if size_data.tile_x_offset >= size_data.reference_grid_width
        || size_data.tile_y_offset >= size_data.reference_grid_height
    {
        return Err("invalid image dimensions");
    }

    // The tile grid offsets (XTOsiz, YTOsiz) are constrained to be no greater than the
    // image area offsets (B-3).
    if size_data.tile_x_offset > size_data.image_area_x_offset
        || size_data.tile_y_offset > size_data.image_area_y_offset
    {
        return Err("tile offsets are invalid");
    }

    // Also, the tile size plus the tile offset shall be greater than the image area offset.
    // This ensures that the first tile (tile 0) will contain at least one reference grid point
    // from the image area (B-4).
    if size_data.tile_x_offset + size_data.tile_width <= size_data.image_area_x_offset
        || size_data.tile_y_offset + size_data.tile_height <= size_data.image_area_y_offset
    {
        return Err("tile offsets are invalid");
    }

    for comp in &size_data.component_sizes {
        if comp.precision == 0 || comp.vertical_resolution == 0 || comp.horizontal_resolution == 0 {
            return Err("invalid component metadata");
        }

        if comp.precision > 8 {
            return Err(
                "unsupported component precision: only components up to 8 bits are handled",
            );
        }
    }

    Ok(size_data)
}

fn size_marker_inner(reader: &mut Reader) -> Option<SizeData> {
    // Length.
    let _ = reader.read_u16()?;
    // Decoder capabilities.
    let _ = reader.read_u16()?;

    let xsiz = reader.read_u32()?;
    let ysiz = reader.read_u32()?;
    let x_osiz = reader.read_u32()?;
    let y_osiz = reader.read_u32()?;
    let xt_siz = reader.read_u32()?;
    let yt_siz = reader.read_u32()?;
    let xto_siz = reader.read_u32()?;
    let yto_siz = reader.read_u32()?;
    let csiz = reader.read_u16()?;

    let mut components = Vec::with_capacity(csiz as usize);
    for _ in 0..csiz {
        let ssiz = reader.read_byte()?;
        let x_rsiz = reader.read_byte()?;
        let y_rsiz = reader.read_byte()?;

        let precision = (ssiz & 0x7F) + 1;
        let is_signed = (ssiz & 0x80) != 0;

        if (x_rsiz != 1 || y_rsiz != 1) && (x_osiz != 0 || y_osiz != 0) {
            // Those are probably very rare. Let's wait until we have a test case
            // before attempting to implement it.
            return None;
        }

        components.push(ComponentSizeInfo {
            precision,
            _is_signed: is_signed,
            horizontal_resolution: x_rsiz,
            vertical_resolution: y_rsiz,
        });
    }

    Some(SizeData {
        reference_grid_width: xsiz,
        reference_grid_height: ysiz,
        image_area_x_offset: x_osiz,
        image_area_y_offset: y_osiz,
        tile_width: xt_siz,
        tile_height: yt_siz,
        tile_x_offset: xto_siz,
        tile_y_offset: yto_siz,
        component_sizes: components,
    })
}

fn coding_style_parameters(
    reader: &mut Reader,
    coding_style: &CodingStyleFlags,
) -> Option<CodingStyleParameters> {
    let num_decomposition_levels = reader.read_byte()? as u16;
    let num_resolution_levels = num_decomposition_levels.checked_add(1)?;
    let code_block_width = reader.read_byte()? + 2;
    let code_block_height = reader.read_byte()? + 2;
    let code_block_style = CodeBlockStyle::from_u8(reader.read_byte()?);
    let transformation = WaveletTransform::from_u8(reader.read_byte()?).ok()?;

    let mut precinct_exponents = Vec::new();
    if coding_style.has_precincts() {
        // "Entropy coder with precincts defined below."
        for _ in 0..num_resolution_levels {
            // Table A.21.
            let precinct_size = reader.read_byte()?;
            let width_exp = precinct_size & 0xF;
            let height_exp = precinct_size >> 4;
            precinct_exponents.push((width_exp, height_exp));
        }
    } else {
        // "Entropy coder, precincts with PPx = 15 and PPy = 15"
        for _ in 0..num_resolution_levels {
            precinct_exponents.push((15, 15));
        }
    }

    Some(CodingStyleParameters {
        num_decomposition_levels,
        num_resolution_levels,
        code_block_width,
        code_block_height,
        code_block_style,
        transformation,
        precinct_exponents,
    })
}

/// COM Marker (A.9.2).
fn com_marker(reader: &mut Reader) -> Option<()> {
    skip_marker_segment(reader)
}

/// TLM marker (A.7.1).
fn tlm_marker(reader: &mut Reader) -> Option<()> {
    skip_marker_segment(reader)
}

/// RGN marker (A.6.3).
fn rgn_marker(reader: &mut Reader) -> Option<()> {
    skip_marker_segment(reader)
}

pub(crate) fn skip_marker_segment(reader: &mut Reader) -> Option<()> {
    let length = reader.read_u16()?.checked_sub(2)?;
    reader.skip_bytes(length as usize)?;

    Some(())
}

/// COD marker (A.6.1).
pub(crate) fn cod_marker(reader: &mut Reader) -> Option<CodingStyleDefault> {
    // Length.
    let _ = reader.read_u16()?;

    let coding_style_flags = CodingStyleFlags::from_u8(reader.read_byte()?);
    let progression_order = ProgressionOrder::from_u8(reader.read_byte()?).ok()?;

    let num_layers = reader.read_u16()?;
    let mct = reader.read_byte()? == 1;

    let coding_style_parameters = coding_style_parameters(reader, &coding_style_flags)?;

    Some(CodingStyleDefault {
        progression_order,
        num_layers,
        mct,
        component_parameters: CodingStyleComponent {
            flags: coding_style_flags,
            parameters: coding_style_parameters,
        },
    })
}

/// COC marker (A.6.2).
pub(crate) fn coc_marker(reader: &mut Reader, csiz: u16) -> Option<(u16, CodingStyleComponent)> {
    // Length.
    let _ = reader.read_u16()?;

    let component_index = if csiz < 257 {
        reader.read_byte()? as u16
    } else {
        reader.read_u16()?
    };
    let coding_style = CodingStyleFlags::from_u8(reader.read_byte()?);

    // Read SPcoc - coding style parameters (same structure as SPcod from COD)
    let parameters = coding_style_parameters(reader, &coding_style)?;

    let coc = CodingStyleComponent {
        flags: coding_style,
        parameters,
    };

    Some((component_index, coc))
}

/// QCD marker (A.6.4).
pub(crate) fn qcd_marker(reader: &mut Reader) -> Option<QuantizationInfo> {
    // Length.
    let length = reader.read_u16()?;

    let sqcd_val = reader.read_byte()?;
    let quantization_style = QuantizationStyle::from_u8(sqcd_val & 0x1F).ok()?;
    let guard_bits = (sqcd_val >> 5) & 0x07;

    let remaining_bytes = (length - 3) as usize;

    let mut parameters = quantization_parameters(reader, quantization_style, remaining_bytes)?;
    parameters.guard_bits = guard_bits;

    Some(parameters)
}

/// QCC marker (A.6.5).
pub(crate) fn qcc_marker(reader: &mut Reader, csiz: u16) -> Option<(u16, QuantizationInfo)> {
    let length = reader.read_u16()?;

    let component_index = if csiz < 257 {
        reader.read_byte()? as u16
    } else {
        reader.read_u16()?
    };

    let sqcc_val = reader.read_byte()?;
    let quantization_style = QuantizationStyle::from_u8(sqcc_val & 0x1F).ok()?;
    let guard_bits = (sqcc_val >> 5) & 0x07;

    let component_index_size = if csiz < 257 { 1 } else { 2 };
    let remaining_bytes = (length - 2 - component_index_size - 1) as usize;

    let mut parameters = quantization_parameters(reader, quantization_style, remaining_bytes)?;
    parameters.guard_bits = guard_bits;

    Some((component_index, parameters))
}

fn quantization_parameters(
    reader: &mut Reader,
    quantization_style: QuantizationStyle,
    remaining_bytes: usize,
) -> Option<QuantizationInfo> {
    let mut step_sizes = Vec::new();

    let irreversible = |val: u16| {
        let exponent = val >> 11;
        let mantissa = val & ((1 << 11) - 1);

        StepSize { exponent, mantissa }
    };

    match quantization_style {
        QuantizationStyle::NoQuantization => {
            // 8 bits per band (5 bits exponent, 3 bits reserved)
            for _ in 0..remaining_bytes {
                let value = reader.read_byte()? as u16;
                step_sizes.push(StepSize {
                    // Unused.
                    mantissa: 0,
                    exponent: (value >> 3),
                });
            }
        }
        QuantizationStyle::ScalarDerived => {
            let value = reader.read_u16()?;
            step_sizes.push(irreversible(value));
        }
        QuantizationStyle::ScalarExpounded => {
            let num_bands = remaining_bytes / 2;
            for _ in 0..num_bands {
                let value = reader.read_u16()?;

                step_sizes.push(irreversible(value));
            }
        }
    }

    Some(QuantizationInfo {
        quantization_style,
        guard_bits: 0, // Will be set by caller.
        step_sizes,
    })
}

// TODO: Use this
fn _skip_code(marker_code: u8) -> bool {
    // All markers with the marker code between 0xFF30 and 0xFF3F have no marker
    // segment parameters. They shall be skipped by the decoder.
    (0x30..=0x3F).contains(&marker_code)
}

pub(crate) trait ReaderExt: Clone {
    fn read_marker(&mut self) -> Result<u8, &'static str>;
    fn peek_marker(&mut self) -> Option<u8> {
        self.clone().read_marker().ok()
    }
}

impl ReaderExt for Reader<'_> {
    fn read_marker(&mut self) -> Result<u8, &'static str> {
        if self.peek_byte().ok_or("invalid marker")? != 0xFF {
            return Err("invalid marker");
        }

        self.read_byte().unwrap();
        self.read_byte().ok_or("invalid marker")
    }
}

#[allow(unused)]
/// Marker codes (Table A.2).
pub(crate) mod markers {
    /// Start of codestream - 'SOC'.
    pub(crate) const SOC: u8 = 0x4F;
    /// Start of tile-part - 'SOT'.
    pub(crate) const SOT: u8 = 0x90;
    /// Start of data - 'SOD'.
    pub(crate) const SOD: u8 = 0x93;
    /// End of codestream - 'EOC'.
    pub(crate) const EOC: u8 = 0xD9;

    /// Image and tile size - 'SIZ'.
    pub(crate) const SIZ: u8 = 0x51;

    /// Coding style default - 'COD'.
    pub(crate) const COD: u8 = 0x52;
    /// Coding component - 'COC'.
    pub(crate) const COC: u8 = 0x53;
    /// Region-of-interest - 'RGN'.
    pub(crate) const RGN: u8 = 0x5E;
    /// Quantization default - 'QCD'.
    pub(crate) const QCD: u8 = 0x5C;
    /// Quantization component - 'QCC'.
    pub(crate) const QCC: u8 = 0x5D;
    /// Progression order change - 'POC'.
    pub(crate) const POC: u8 = 0x5F;

    /// Tile-part lengths - 'TLM'.
    pub(crate) const TLM: u8 = 0x55;
    /// Packet length, main header - 'PLM'.
    pub(crate) const PLM: u8 = 0x57;
    /// Packet length, tile-part header - 'PLT'.
    pub(crate) const PLT: u8 = 0x58;
    /// Packed packet headers, main header - 'PPM'.
    pub(crate) const PPM: u8 = 0x60;
    /// Packed packet headers, tile-part header - 'PPT'.
    pub(crate) const PPT: u8 = 0x61;

    /// Start of packet - 'SOP'.
    pub(crate) const SOP: u8 = 0x91;
    /// End of packet header - 'EPH'.
    pub(crate) const EPH: u8 = 0x92;

    /// Component registration - 'CRG'.
    pub(crate) const CRG: u8 = 0x63;
    /// Comment - 'COM'.
    pub(crate) const COM: u8 = 0x64;

    pub(crate) fn to_string(marker: u8) -> &'static str {
        match marker {
            // Delimiting markers.
            SOC => "SOC",
            SOT => "SOT",
            SOD => "SOD",
            EOC => "EOC",

            // Fixed information.
            SIZ => "SIZ",

            // Functional markers.
            COD => "COD",
            COC => "COC",
            RGN => "RGN",
            QCD => "QCD",
            QCC => "QCC",
            POC => "POC",

            // Pointer markers.
            TLM => "TLM",
            PLM => "PLM",
            PLT => "PLT",
            PPM => "PPM",
            PPT => "PPT",

            // In-bit-stream markers.
            SOP => "SOP",
            EPH => "EPH",

            // Informational markers.
            CRG => "CRG",
            COM => "COM",

            _ => "UNKNOWN",
        }
    }
}
