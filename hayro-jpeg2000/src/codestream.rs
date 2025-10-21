use hayro_common::byte::Reader;

pub(crate) fn read(stream: &[u8]) -> Result<(), &'static str> {
    let mut reader = Reader::new(stream);

    let marker = reader.read_marker()?;
    if marker != markers::SOC {
        return Err("invalid marker: expected SOC marker");
    }

    let header = read_header(&mut reader)?;

    let mut tiles = vec![];
    tiles.push(read_tile_part(&mut reader, &header).ok_or("failed to read first tile part")?);

    while reader.peek_marker() == Some(markers::SOT) {
        tiles.push(read_tile_part(&mut reader, &header).ok_or("failed to read a tile part")?);
    }

    if reader.read_marker()? != markers::EOC {
        return Err("invalid marker: expected EOC marker");
    }

    Ok(())
}

/// Progression order (Table A.16).
#[derive(Debug, Clone, Copy)]
enum ProgressionOrder {
    Lrcp,
    Rlcp,
    Rpcl,
    Pcrl,
    Cprl,
}

impl ProgressionOrder {
    fn from_u8(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(ProgressionOrder::Lrcp),
            1 => Ok(ProgressionOrder::Rlcp),
            2 => Ok(ProgressionOrder::Rpcl),
            3 => Ok(ProgressionOrder::Pcrl),
            4 => Ok(ProgressionOrder::Cprl),
            _ => Err("invalid progression order"),
        }
    }
}

/// Multiple component transformation type (Table A.17).
#[derive(Debug, Clone, Copy)]
enum MultipleComponentTransform {
    None,
    Used,
}

impl MultipleComponentTransform {
    fn from_u8(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(MultipleComponentTransform::None),
            1 => Ok(MultipleComponentTransform::Used),
            _ => Err("invalid MCT value"),
        }
    }
}

/// Wavelet transformation type (Table A.20).
#[derive(Debug, Clone, Copy)]
enum WaveletTransform {
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
#[derive(Debug, Clone, Copy)]
struct CodingStyleFlags {
    raw: u8,
}

impl CodingStyleFlags {
    fn from_u8(value: u8) -> Self {
        CodingStyleFlags { raw: value }
    }

    fn has_precincts(&self) -> bool {
        (self.raw & 0x01) != 0
    }

    fn uses_sop_markers(&self) -> bool {
        (self.raw & 0x01) != 0
    }

    fn uses_eph_marker(&self) -> bool {
        (self.raw & 0x02) != 0
    }
}

/// Code-block style flags (Table A.19).
#[derive(Debug, Clone, Copy)]
struct CodeBlockStyle {
    /// Selective arithmetic coding bypass.
    selective_arithmetic_coding_bypass: bool,
    /// Reset context probabilities on coding pass boundaries.
    reset_context_probabilities: bool,
    /// Termination on each coding pass.
    termination_on_each_pass: bool,
    /// Vertically causal context.
    vertically_causal_context: bool,
    /// Predictable termination.
    predictable_termination: bool,
    /// Segmentation symbols are used.
    segmentation_symbols: bool,
}

impl CodeBlockStyle {
    fn from_u8(value: u8) -> Self {
        CodeBlockStyle {
            selective_arithmetic_coding_bypass: (value & 0x01) != 0,
            reset_context_probabilities: (value & 0x02) != 0,
            termination_on_each_pass: (value & 0x04) != 0,
            vertically_causal_context: (value & 0x08) != 0,
            predictable_termination: (value & 0x10) != 0,
            segmentation_symbols: (value & 0x20) != 0,
        }
    }
}

#[derive(Debug)]
struct ComponentInfo {
    /// Precision (depth) in bits and sign of the component samples.
    precision: u8,
    is_signed: bool,
    /// Horizontal separation of a sample with respect to the reference grid.
    x_rsiz: u8,
    /// Vertical separation of a sample with respect to the reference grid.
    y_rsiz: u8,
}

/// Quantization style (Table A.28).
#[derive(Debug, Clone, Copy)]
enum QuantizationStyle {
    /// No quantization.
    NoQuantization = 0,
    /// Scalar derived (values signalled for N/LL sub-band only).
    ScalarDerived = 1,
    /// Scalar expounded (values signalled for each sub-band).
    ScalarExpounded = 2,
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

/// Common coding style parameters (SPcod/SPcoc).
#[derive(Clone, Debug)]
struct CodingStyleParameters {
    num_decomposition_levels: u8,
    code_block_width: u8,
    code_block_height: u8,
    code_block_style: CodeBlockStyle,
    transformation: WaveletTransform,
    precinct_sizes: Vec<u8>,
}

/// Common quantization parameters (SPqcd/SPqcc).
#[derive(Clone, Debug)]
struct QuantizationParameters {
    quantization_style: QuantizationStyle,
    guard_bits: u8,
    step_sizes: Vec<u16>,
}

#[derive(Debug, Clone)]
struct CodingStyleInfo {
    style: CodingStyleFlags,
    progression_order: ProgressionOrder,
    num_layers: u16,
    mct: MultipleComponentTransform,
    parameters: CodingStyleParameters,
}

#[derive(Clone, Debug)]
struct CodingStyleComponent {
    scoc: CodingStyleFlags,
    parameters: CodingStyleParameters,
}

#[derive(Debug, Clone)]
struct QuantizationInfo {
    parameters: QuantizationParameters,
}

#[derive(Clone, Debug)]
struct QuantizationComponent {
    parameters: QuantizationParameters,
}

#[derive(Debug)]
struct SizeData {
    /// Decoder capabilities.
    rsiz: u16,
    /// Width of the reference grid.
    xsiz: u32,
    /// Height of the reference grid.
    ysiz: u32,
    /// Horizontal offset from the origin of the reference grid to the left side of the image area.
    x_osiz: u32,
    /// Vertical offset from the origin of the reference grid to the top side of the image area.
    y_osiz: u32,
    /// Width of one reference tile with respect to the reference grid.
    xt_siz: u32,
    /// Height of one reference tile with respect to the reference grid.
    yt_siz: u32,
    /// Horizontal offset from the origin of the reference grid to the left side of the first tile.
    xto_siz: u32,
    /// Vertical offset from the origin of the reference grid to the top side of the first tile.
    yto_siz: u32,
    /// Number of components in the image.
    csiz: u16,
    /// Component information.
    components: Vec<ComponentInfo>,
}

#[derive(Debug)]
struct Header {
    size_data: SizeData,
    cod_components: Vec<CodingStyleInfo>,
    qcd_components: Vec<QuantizationInfo>,
}

fn read_header(reader: &mut Reader) -> Result<Header, &'static str> {
    if reader.read_marker()? != markers::SIZ {
        return Err("expected SIZ marker after SOC");
    }

    let size_data = size_marker(reader).ok_or("failed to read SIZ marker")?;

    let mut cod = None;
    let mut qcd = None;

    // Initialize vectors for component-specific styles
    let num_components = size_data.csiz as usize;
    let mut cod_components = vec![None; num_components];
    let mut qcd_components = vec![None; num_components];

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
                    coc_marker(reader, size_data.csiz).ok_or("failed to read COC marker")?;
                cod_components[component_index as usize] = Some(coc);
            }
            markers::QCD => {
                reader.read_marker()?;
                qcd = Some(qcd_marker(reader).ok_or("failed to read QCD marker")?);

                eprintln!("{:?}", qcd);
            }
            markers::QCC => {
                reader.read_marker()?;
                let (component_index, qcc) =
                    qcc_marker(reader, size_data.csiz).ok_or("failed to read QCC marker")?;
                qcd_components[component_index as usize] = Some(qcc);
            }
            m => {
                panic!("marker: {}", markers::to_string(m));
            }
        }
    }

    let cod = cod.ok_or("missing COD marker")?;
    let qcd = qcd.ok_or("missing QCD marker")?;

    Ok(Header {
        size_data,
        cod_components: cod_components
            .into_iter()
            .map(|coc| {
                let mut cloned = cod.clone();

                // COC takes precedence over COD if available.
                if let Some(coc) = coc {
                    cloned.style = coc.scoc;
                    cloned.parameters = coc.parameters;
                }

                cloned
            })
            .collect(),
        qcd_components: qcd_components
            .into_iter()
            .map(|c| c.unwrap_or(qcd.clone()))
            .collect(),
    })
}

struct TilePart<'a> {
    header: TilePartHeader,
    data: &'a [u8],
}

fn read_tile_part<'a>(reader: &mut Reader<'a>, header: &Header) -> Option<TilePart<'a>> {
    if reader.read_marker().ok()? != markers::SOT {
        return None;
    }

    let (mut tile_part_reader, header) = {
        let sot_marker = sot_marker(reader)?;
        let data = if sot_marker.tile_part_length == 0 {
            // Data goes until EOC
            let data = reader.tail()?;
            reader.jump_to_end();

            data
        } else {
            // Subtract 12 to account for the marker length.
            let length = (sot_marker.tile_part_length as usize).checked_sub(12)?;

            let data = reader.tail()?.get(..length)?;
            // Skip to the very end in the original reader.
            reader.skip_bytes(length)?;

            data
        };

        (Reader::new(data), sot_marker)
    };

    loop {
        match tile_part_reader.peek_marker()? {
            markers::SOD => {
                tile_part_reader.read_marker().ok()?;
                break;
            }
            markers::EOC => break,
            m => {
                panic!("marker: {}", markers::to_string(m));
            }
        }
    }

    Some(TilePart {
        data: tile_part_reader.tail()?,
        header,
    })
}

struct TilePartHeader {
    tile_index: u16,
    tile_part_length: u32,
    tile_part_index: u8,
    num_tile_parts: u8,
}

fn sot_marker(reader: &mut Reader) -> Option<TilePartHeader> {
    let _lsot = reader.read_u16()?;
    let tile_index = reader.read_u16()?;
    let tile_part_length = reader.read_u32()?;
    let tile_part_index = reader.read_byte()?;
    let num_tile_parts = reader.read_byte()?;

    Some(TilePartHeader {
        tile_index,
        tile_part_length,
        tile_part_index,
        num_tile_parts,
    })
}

fn size_marker(reader: &mut Reader) -> Option<SizeData> {
    let _lsiz = reader.read_u16()?;

    // Read SIZ parameters
    let rsiz = reader.read_u16()?;
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

        components.push(ComponentInfo {
            precision,
            is_signed,
            x_rsiz,
            y_rsiz,
        });
    }

    Some(SizeData {
        rsiz,
        xsiz,
        ysiz,
        x_osiz,
        y_osiz,
        xt_siz,
        yt_siz,
        xto_siz,
        yto_siz,
        csiz,
        components,
    })
}

/// Reads common coding style parameters (SPcod/SPcoc).
fn read_coding_style_parameters(
    reader: &mut Reader,
    coding_style: &CodingStyleFlags,
) -> Option<CodingStyleParameters> {
    let num_decomposition_levels = reader.read_byte()?;
    let code_block_width = reader.read_byte()?;
    let code_block_height = reader.read_byte()?;

    let code_block_style_val = reader.read_byte()?;
    let code_block_style = CodeBlockStyle::from_u8(code_block_style_val);

    let transformation_val = reader.read_byte()?;
    let transformation = WaveletTransform::from_u8(transformation_val).ok()?;

    // Check if precinct sizes are present
    let mut precinct_sizes = Vec::new();
    if coding_style.has_precincts() {
        // Read precinct sizes for each resolution level (num_decomposition_levels + 1)
        for _ in 0..=num_decomposition_levels {
            let precinct_size = reader.read_byte()?;
            precinct_sizes.push(precinct_size);
        }
    }

    Some(CodingStyleParameters {
        num_decomposition_levels,
        code_block_width,
        code_block_height,
        code_block_style,
        transformation,
        precinct_sizes,
    })
}

fn cod_marker(reader: &mut Reader) -> Option<CodingStyleInfo> {
    // Length.
    let _ = reader.read_u16()?;

    let coding_style = CodingStyleFlags::from_u8(reader.read_byte()?);
    let progression_order = ProgressionOrder::from_u8(reader.read_byte()?).ok()?;

    let num_layers = reader.read_u16()?;
    let mct = MultipleComponentTransform::from_u8(reader.read_byte()?).ok()?;

    let coding_style_parameters = read_coding_style_parameters(reader, &coding_style)?;

    Some(CodingStyleInfo {
        style: coding_style,
        progression_order,
        num_layers,
        mct,
        parameters: coding_style_parameters,
    })
}

fn coc_marker(reader: &mut Reader, csiz: u16) -> Option<(u16, CodingStyleComponent)> {
    // Length.
    let _ = reader.read_u16()?;

    let component_index = if csiz < 257 {
        reader.read_byte()? as u16
    } else {
        reader.read_u16()?
    };
    let coding_style = CodingStyleFlags::from_u8(reader.read_byte()?);

    // Read SPcoc - coding style parameters (same structure as SPcod from COD)
    let parameters = read_coding_style_parameters(reader, &coding_style)?;

    let coc = CodingStyleComponent { scoc: coding_style, parameters };

    Some((component_index, coc))
}

/// Reads common quantization parameters (SPqcd/SPqcc).
fn read_quantization_parameters(
    reader: &mut Reader,
    quantization_style: QuantizationStyle,
    remaining_bytes: usize,
) -> Option<QuantizationParameters> {
    let mut step_sizes = Vec::new();

    match quantization_style {
        QuantizationStyle::NoQuantization => {
            // 8 bits per band (5 bits exponent, 3 bits reserved)
            for _ in 0..remaining_bytes {
                let value = reader.read_byte()? as u16;
                step_sizes.push(value);
            }
        }
        QuantizationStyle::ScalarDerived => {
            let value = reader.read_u16()?;
            step_sizes.push(value);
        }
        QuantizationStyle::ScalarExpounded => {
            // 16 bits per band
            let num_bands = remaining_bytes / 2;
            for _ in 0..num_bands {
                let value = reader.read_u16()?;
                step_sizes.push(value);
            }
        }
    }

    Some(QuantizationParameters {
        quantization_style,
        guard_bits: 0, // Will be set by caller
        step_sizes,
    })
}

fn qcd_marker(reader: &mut Reader) -> Option<QuantizationInfo> {
    let lqcd = reader.read_u16()?;

    // Read Sqcd - quantization style and guard bits
    let sqcd_val = reader.read_byte()?;
    let quantization_style = QuantizationStyle::from_u8(sqcd_val & 0x1F).ok()?;
    let guard_bits = (sqcd_val >> 5) & 0x07;

    // Calculate remaining bytes for step sizes
    let remaining_bytes = (lqcd - 3) as usize;

    // Read SPqcd - quantization step size values
    let mut parameters = read_quantization_parameters(reader, quantization_style, remaining_bytes)?;
    parameters.guard_bits = guard_bits;

    Some(QuantizationInfo { parameters })
}

fn qcc_marker(reader: &mut Reader, csiz: u16) -> Option<(u16, QuantizationInfo)> {
    let lqcc = reader.read_u16()?;

    // Read Cqcc - component index (8 or 16 bits depending on number of components)
    let component_index = if csiz < 257 {
        reader.read_byte()? as u16
    } else {
        reader.read_u16()?
    };

    // Read Sqcc - quantization style and guard bits for this component
    let sqcc_val = reader.read_byte()?;
    let quantization_style = QuantizationStyle::from_u8(sqcc_val & 0x1F).ok()?;
    let guard_bits = (sqcc_val >> 5) & 0x07;

    // Calculate remaining bytes for step sizes
    let component_index_size = if csiz < 257 { 1 } else { 2 };
    let remaining_bytes = (lqcc - 2 - component_index_size - 1) as usize; // Total length - Lqcc (2) - Cqcc - Sqcc (1)

    // Read SPqcc - quantization step size values
    let mut parameters = read_quantization_parameters(reader, quantization_style, remaining_bytes)?;
    parameters.guard_bits = guard_bits;

    let qcc = QuantizationInfo { parameters };

    Some((component_index, qcc))
}

fn skip_code(marker_code: u8) -> bool {
    // All markers with the marker code between 0xFF30 and 0xFF3F have no marker
    // segment parameters. They shall be skipped by the decoder.
    marker_code >= 0x30 && marker_code <= 0x3F
}

trait ReaderExt: Clone {
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

/// Marker codes (Table A.2).
mod markers {
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
