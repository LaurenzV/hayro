use hayro_common::byte::Reader;

pub(crate) fn read(stream: &[u8]) -> Result<(), &'static str> {
    let mut reader = Reader::new(stream);

    while !reader.at_end() {
        let marker_prefix = reader.read_byte().ok_or("failed to read marker prefix")?;

        if marker_prefix != 0xFF {
            return Err("invalid marker: expected 0xFF prefix");
        }

        let marker_code = reader.read_byte().ok_or("failed to read marker code")?;

        println!(
            "Found marker: 0xFF{:02X} ({})",
            marker_code,
            markers::to_string(marker_code)
        );
        
        match marker_code {
            markers::SOC => {
                read_header(&mut reader)?;
            }
            markers::SOD => {
                // Start of data - remaining bytes are compressed image data
                println!("  -> Remaining bytes are compressed image data");
                reader.jump_to_end();
                break;
            }
            markers::EOC => {
                // End of codestream
                println!("  -> End of codestream");
                break;
            }
            i if skip_code(i) => {
                // Delimiter markers with no parameters
                continue;
            }
            _ => {
                // Marker segments with length parameter - read length and skip
                let length = reader.read_u16().ok_or("failed to read marker segment length")?;
                println!("  -> Segment length: {} bytes (skipping)", length);

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

struct ComponentInfo {
    /// Precision (depth) in bits and sign of the component samples
    precision: u8,
    is_signed: bool,
    /// Horizontal separation of a sample with respect to the reference grid
    x_rsiz: u8,
    /// Vertical separation of a sample with respect to the reference grid
    y_rsiz: u8,
}

struct SizeData {
    /// Decoder capabilities
    rsiz: u16,
    /// Width of the reference grid
    xsiz: u32,
    /// Height of the reference grid
    ysiz: u32,
    /// Horizontal offset from the origin of the reference grid to the left side of the image area
    x_osiz: u32,
    /// Vertical offset from the origin of the reference grid to the top side of the image area
    y_osiz: u32,
    /// Width of one reference tile with respect to the reference grid
    xt_siz: u32,
    /// Height of one reference tile with respect to the reference grid
    yt_siz: u32,
    /// Horizontal offset from the origin of the reference grid to the left side of the first tile
    xto_siz: u32,
    /// Vertical offset from the origin of the reference grid to the top side of the first tile
    yto_siz: u32,
    /// Number of components in the image
    csiz: u16,
    /// Component information
    components: Vec<ComponentInfo>,
}

fn read_header(reader: &mut Reader) -> Result<SizeData, &'static str> {
    let marker_prefix = reader.read_byte().ok_or("failed to read marker prefix")?;
    
    if marker_prefix != 0xFF {
        return Err("invalid marker: expected 0xFF prefix");
    }

    let marker_code = reader.read_byte().ok_or("failed to read marker code")?;
    if marker_code != markers::SIZ {
        return Err("expected SIZ marker after SOC");
    }

    read_size(reader)
}

fn read_size(reader: &mut Reader) -> Result<SizeData, &'static str> {
    let _lsiz = reader.read_u16()
        .ok_or("failed to read SIZ length")?;

    // Read SIZ parameters
    let rsiz = reader.read_u16().ok_or("failed to read Rsiz")?;
    let xsiz = reader.read_u32().ok_or("failed to read Xsiz")?;
    let ysiz = reader.read_u32().ok_or("failed to read Ysiz")?;
    let x_osiz = reader.read_u32().ok_or("failed to read XOsiz")?;
    let y_osiz = reader.read_u32().ok_or("failed to read YOsiz")?;
    let xt_siz = reader.read_u32().ok_or("failed to read XTsiz")?;
    let yt_siz = reader.read_u32().ok_or("failed to read YTsiz")?;
    let xto_siz = reader.read_u32().ok_or("failed to read XTOsiz")?;
    let yto_siz = reader.read_u32().ok_or("failed to read YTOsiz")?;
    let csiz = reader.read_u16().ok_or("failed to read Csiz")?;

    let mut components = Vec::with_capacity(csiz as usize);
    for _ in 0..csiz {
        let ssiz = reader.read_byte().ok_or("failed to read Ssiz")?;
        let x_rsiz = reader.read_byte().ok_or("failed to read XRsiz")?;
        let y_rsiz = reader.read_byte().ok_or("failed to read YRsiz")?;

        let precision = (ssiz & 0x7F) + 1;
        let is_signed = (ssiz & 0x80) != 0;

        components.push(ComponentInfo {
            precision,
            is_signed,
            x_rsiz,
            y_rsiz,
        });
    }

    Ok(SizeData {
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

fn skip_code(marker_code: u8) -> bool {
    // All markers with the marker code between 0xFF30 and 0xFF3F have no marker
    // segment parameters. They shall be skipped by the decoder.
    marker_code >= 0x30 && marker_code <= 0x3F
}

/// Table A.2: The different marker segments.
mod markers {
    /// Start of codestream - 'SOC'
    pub(crate) const SOC: u8 = 0x4F;
    /// Start of tile-part - 'SOT'
    pub(crate) const SOT: u8 = 0x90;
    /// Start of data - 'SOD'
    pub(crate) const SOD: u8 = 0x93;
    /// End of codestream - 'EOC'
    pub(crate) const EOC: u8 = 0xD9;

    /// Image and tile size - 'SIZ'
    pub(crate) const SIZ: u8 = 0x51;

    /// Coding style default - 'COD'
    pub(crate) const COD: u8 = 0x52;
    /// Coding component - 'COC'
    pub(crate) const COC: u8 = 0x53;
    /// Region-of-interest - 'RGN'
    pub(crate) const RGN: u8 = 0x5E;
    /// Quantization default - 'QCD'
    pub(crate) const QCD: u8 = 0x5C;
    /// Quantization component - 'QCC'
    pub(crate) const QCC: u8 = 0x5D;
    /// Progression order change - 'POC'
    pub(crate) const POC: u8 = 0x5F;

    /// Tile-part lengths - 'TLM'
    pub(crate) const TLM: u8 = 0x55;
    /// Packet length, main header - 'PLM'
    pub(crate) const PLM: u8 = 0x57;
    /// Packet length, tile-part header - 'PLT'
    pub(crate) const PLT: u8 = 0x58;
    /// Packed packet headers, main header - 'PPM'
    pub(crate) const PPM: u8 = 0x60;
    /// Packed packet headers, tile-part header - 'PPT'
    pub(crate) const PPT: u8 = 0x61;

    /// Start of packet - 'SOP'
    pub(crate) const SOP: u8 = 0x91;
    /// End of packet header - 'EPH'
    pub(crate) const EPH: u8 = 0x92;

    /// Component registration - 'CRG'
    pub(crate) const CRG: u8 = 0x63;
    /// Comment - 'COM'
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
