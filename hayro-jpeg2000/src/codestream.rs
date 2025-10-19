use hayro_common::byte::Reader;

pub(crate) fn read(stream: &[u8]) -> Result<(), &'static str> {
    let mut reader = Reader::new(stream);

    // Loop through all markers in the codestream
    while !reader.at_end() {
        // Read marker: 2 bytes (0xFF + marker code)
        let marker_prefix = reader.read_byte().ok_or("failed to read marker prefix")?;

        if marker_prefix != 0xFF {
            return Err("invalid marker: expected 0xFF prefix");
        }

        let marker_code = reader.read_byte().ok_or("failed to read marker code")?;

        println!("Found marker: 0xFF{:02X} ({})", marker_code, markers::to_string(marker_code));

        // Check if this is a delimiter marker (no parameters)
        // Also, markers in range 0xFF30-0xFF3F are reserved for markers without parameters
        let is_delimiter = matches!(marker_code, markers::SOC | markers::SOD | markers::EOC)
            || (marker_code >= 0x30 && marker_code <= 0x3F);

        if is_delimiter {
            // Delimiter markers - no parameters to skip
            println!("  -> Delimiter marker (no parameters)");

            // Special handling for SOD - everything after is compressed image data
            if marker_code == markers::SOD {
                println!("  -> Remaining bytes are compressed image data");
                // For now, just skip to end
                reader.jump_to_end();
                break;
            }
        } else {
            // Marker segment with length parameter
            // Length is 2 bytes, includes itself but not the marker
            let length = reader.read_u16().ok_or("failed to read marker segment length")?;

            println!("  -> Segment length: {} bytes", length);

            // Skip the parameter data (length - 2 because length includes the 2-byte length field)
            if length < 2 {
                return Err("invalid marker segment length");
            }

            let param_length = (length - 2) as usize;
            reader.read_bytes(param_length).ok_or("failed to skip marker segment parameters")?;
        }
    }

    Ok(())
}

/// Table A.2: The different marker segments.
mod markers {
    /// Start of codestream - 'SOC'
    pub const SOC: u8 = 0x4F;
    /// Start of tile-part - 'SOT'
    pub const SOT: u8 = 0x90;
    /// Start of data - 'SOD'
    pub const SOD: u8 = 0x93;
    /// End of codestream - 'EOC'
    pub const EOC: u8 = 0xD9;

    /// Image and tile size - 'SIZ'
    pub const SIZ: u8 = 0x51;

    /// Coding style default - 'COD'
    pub const COD: u8 = 0x52;
    /// Coding component - 'COC'
    pub const COC: u8 = 0x53;
    /// Region-of-interest - 'RGN'
    pub const RGN: u8 = 0x5E;
    /// Quantization default - 'QCD'
    pub const QCD: u8 = 0x5C;
    /// Quantization component - 'QCC'
    pub const QCC: u8 = 0x5D;
    /// Progression order change - 'POC'
    pub const POC: u8 = 0x5F;

    /// Tile-part lengths - 'TLM'
    pub const TLM: u8 = 0x55;
    /// Packet length, main header - 'PLM'
    pub const PLM: u8 = 0x57;
    /// Packet length, tile-part header - 'PLT'
    pub const PLT: u8 = 0x58;
    /// Packed packet headers, main header - 'PPM'
    pub const PPM: u8 = 0x60;
    /// Packed packet headers, tile-part header - 'PPT'
    pub const PPT: u8 = 0x61;

    /// Start of packet - 'SOP'
    pub const SOP: u8 = 0x91;
    /// End of packet header - 'EPH'
    pub const EPH: u8 = 0x92;

    /// Component registration - 'CRG'
    pub const CRG: u8 = 0x63;
    /// Comment - 'COM'
    pub const COM: u8 = 0x64;
    
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