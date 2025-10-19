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
            markers::SOC => read_header(&mut reader)?,
            i if skip_code(i) => continue,
            _ => unimplemented!()
        };
    }

    Ok(())
}

struct HeaderData {
    
}

fn read_header(reader: &mut Reader) -> Result<HeaderData, &'static str> {
    
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
