use hayro_common::byte::Reader;

pub(crate) fn read(stream: &[u8]) -> Result<(), &'static str> {
    let mut reader = Reader::new(stream);

    while !reader.at_end() {
        let marker_prefix = reader.read_byte().ok_or("failed to read marker prefix")?;

        if marker_prefix != 0xFF {
            return Err("invalid marker: expected 0xFF prefix");
        }

        let marker_code = reader.read_byte().ok_or("failed to read marker code")?;

        match marker_code {
            markers::SOC => {
                read_header(&mut reader)?;
            }
            markers::SOD => {
                // Start of data - remaining bytes are compressed image data
                reader.jump_to_end();
                break;
            }
            markers::EOC => {
                // End of codestream
                break;
            }
            i if skip_code(i) => {
                // Delimiter markers with no parameters
                continue;
            }
            _ => {
                // Marker segments with length parameter - read length and skip
                let length = reader.read_u16().ok_or("failed to read marker segment length")?;

                if length < 2 {
                    return Err("invalid marker segment length");
                }

                let param_length = (length - 2) as usize;
                reader.read_bytes(param_length).ok_or("failed to skip marker segment parameters")?;
            }
        };
    }

    Ok(())
}

/// Progression order (Table A.16).
#[derive(Debug, Clone, Copy)]
enum ProgressionOrder {
    /// Layer-resolution level-component-position progression.
    LRCP = 0,
    /// Resolution level-layer-component-position progression.
    RLCP = 1,
    /// Resolution level-position-component-layer progression.
    RPCL = 2,
    /// Position-component-resolution level-layer progression.
    PCRL = 3,
    /// Component-position-resolution level-layer progression.
    CPRL = 4,
}

impl ProgressionOrder {
    fn from_u8(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(ProgressionOrder::LRCP),
            1 => Ok(ProgressionOrder::RLCP),
            2 => Ok(ProgressionOrder::RPCL),
            3 => Ok(ProgressionOrder::PCRL),
            4 => Ok(ProgressionOrder::CPRL),
            _ => Err("invalid progression order"),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            ProgressionOrder::LRCP => "LRCP (Layer-resolution level-component-position)",
            ProgressionOrder::RLCP => "RLCP (Resolution level-layer-component-position)",
            ProgressionOrder::RPCL => "RPCL (Resolution level-position-component-layer)",
            ProgressionOrder::PCRL => "PCRL (Position-component-resolution level-layer)",
            ProgressionOrder::CPRL => "CPRL (Component-position-resolution level-layer)",
        }
    }
}

/// Multiple component transformation type (Table A.17).
#[derive(Debug, Clone, Copy)]
enum MultipleComponentTransform {
    /// No multiple component transformation specified.
    None = 0,
    /// Component transformation used (irreversible or reversible).
    Used = 1,
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
    /// 9-7 irreversible filter.
    Irreversible97 = 0,
    /// 5-3 reversible filter.
    Reversible53 = 1,
}

impl WaveletTransform {
    fn from_u8(value: u8) -> Result<Self, &'static str> {
        match value {
            0 => Ok(WaveletTransform::Irreversible97),
            1 => Ok(WaveletTransform::Reversible53),
            _ => Err("invalid transformation type"),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            WaveletTransform::Irreversible97 => "9-7 irreversible",
            WaveletTransform::Reversible53 => "5-3 reversible",
        }
    }
}

/// Coding style flags for Scod parameter (Table A.13).
#[derive(Debug, Clone, Copy)]
struct CodingStyle {
    raw: u8,
}

impl CodingStyle {
    fn from_u8(value: u8) -> Self {
        CodingStyle { raw: value }
    }

    /// Returns true if user-defined precincts are used (bit 0).
    fn has_precincts(&self) -> bool {
        (self.raw & 0x01) != 0
    }

    /// Returns true if SOP marker segments may be used (bit 0).
    fn uses_sop_markers(&self) -> bool {
        (self.raw & 0x01) != 0
    }

    /// Returns true if EPH marker shall be used (bit 1).
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

struct ComponentInfo {
    /// Precision (depth) in bits and sign of the component samples.
    precision: u8,
    is_signed: bool,
    /// Horizontal separation of a sample with respect to the reference grid.
    x_rsiz: u8,
    /// Vertical separation of a sample with respect to the reference grid.
    y_rsiz: u8,
}

/// Common coding style parameters (SPcod/SPcoc).
#[derive(Clone, Debug)]
struct CodingStyleParameters {
    /// Number of decomposition levels.
    num_decomposition_levels: u8,
    /// Code-block width exponent offset value (xcb).
    code_block_width: u8,
    /// Code-block height exponent offset value (ycb).
    code_block_height: u8,
    /// Code-block style.
    code_block_style: CodeBlockStyle,
    /// Wavelet transformation used.
    transformation: WaveletTransform,
    /// Precinct sizes for each resolution level (if present).
    precinct_sizes: Vec<u8>,
}

struct CodingStyleDefault {
    /// Coding style for all components.
    scod: CodingStyle,
    /// Progression order.
    progression_order: ProgressionOrder,
    /// Number of layers.
    num_layers: u16,
    /// Multiple component transformation usage.
    mct: MultipleComponentTransform,
    /// Common coding style parameters (SPcod).
    parameters: CodingStyleParameters,
}

#[derive(Clone)]
struct CodingStyleComponent {
    /// Coding style for this component.
    scoc: CodingStyle,
    /// Common coding style parameters (SPcoc).
    parameters: CodingStyleParameters,
}

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

fn read_header(reader: &mut Reader) -> Result<SizeData, &'static str> {
    if reader.read_marker()? != markers::SIZ {
        return Err("expected SIZ marker after SOC");
    }

    let size_data = size_marker(reader).ok_or("failed to read SIZ marker")?;

    let mut _cod = None;
    let mut _qcd: Option<()> = None;

    // Initialize vector for component-specific coding styles
    let num_components = size_data.csiz as usize;
    let mut _component_coding_styles: Vec<Option<CodingStyleComponent>> = vec![None; num_components];

    loop {
        match reader.peek_marker()? {
            markers::SOT => break,
            markers::COD => {
                reader.read_marker()?;
                _cod = Some(cod_marker(reader).ok_or("failed to read COD marker")?);
            }
            markers::COC => {
                reader.read_marker()?;
                let (component_index, coc) = coc_marker(reader, size_data.csiz)
                    .ok_or("failed to read COC marker")?;
                _component_coding_styles[component_index as usize] = Some(coc);
            }
            m => {
                panic!("marker: {}", markers::to_string(m));
            }
        }
    }

    Ok(size_data)
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
    coding_style: &CodingStyle,
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

fn cod_marker(reader: &mut Reader) -> Option<CodingStyleDefault> {
    let _lcod = reader.read_u16()?;

    // Read Scod - coding style
    let scod_val = reader.read_byte()?;
    let scod = CodingStyle::from_u8(scod_val);

    // Read SGcod - coding style parameters (32 bits)
    let progression_order_val = reader.read_byte()?;
    let progression_order = ProgressionOrder::from_u8(progression_order_val).ok()?;

    let num_layers = reader.read_u16()?;

    let mct_val = reader.read_byte()?;
    let mct = MultipleComponentTransform::from_u8(mct_val).ok()?;

    // Read SPcod - coding style parameters (variable)
    let parameters = read_coding_style_parameters(reader, &scod)?;

    Some(CodingStyleDefault {
        scod,
        progression_order,
        num_layers,
        mct,
        parameters,
    })
}

fn coc_marker(reader: &mut Reader, csiz: u16) -> Option<(u16, CodingStyleComponent)> {
    let _lcoc = reader.read_u16()?;

    // Read Ccoc - component index (8 or 16 bits depending on number of components)
    let component_index = if csiz < 257 {
        reader.read_byte()? as u16
    } else {
        reader.read_u16()?
    };

    // Read Scoc - coding style for this component
    let scoc_val = reader.read_byte()?;
    let scoc = CodingStyle::from_u8(scoc_val);

    // Read SPcoc - coding style parameters (same structure as SPcod from COD)
    let parameters = read_coding_style_parameters(reader, &scoc)?;

    let coc = CodingStyleComponent {
        scoc,
        parameters,
    };

    Some((component_index, coc))
}

fn skip_code(marker_code: u8) -> bool {
    // All markers with the marker code between 0xFF30 and 0xFF3F have no marker
    // segment parameters. They shall be skipped by the decoder.
    marker_code >= 0x30 && marker_code <= 0x3F
}

trait ReaderExt: Clone {
    fn read_marker(&mut self) -> Result<u8, &'static str>;
    fn peek_marker(&mut self) -> Result<u8, &'static str> {
        self.clone().read_marker()
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

/// Table A.2: The different marker segments.
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
            // Delimiting markers
            SOC => "SOC",
            SOT => "SOT",
            SOD => "SOD",
            EOC => "EOC",

            // Fixed information
            SIZ => "SIZ",

            // Functional markers
            COD => "COD",
            COC => "COC",
            RGN => "RGN",
            QCD => "QCD",
            QCC => "QCC",
            POC => "POC",

            // Pointer markers
            TLM => "TLM",
            PLM => "PLM",
            PLT => "PLT",
            PPM => "PPM",
            PPT => "PPT",

            // In-bit-stream markers
            SOP => "SOP",
            EPH => "EPH",

            // Informational markers
            CRG => "CRG",
            COM => "COM",

            // Unknown marker
            _ => "UNKNOWN",
        }
    }
}
