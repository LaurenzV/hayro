/// Ported from <https://github.com/mozilla/pdf.js/blob/master/src/core/cmap.js>

use std::collections::HashMap;

const MAX_MAP_RANGE: u32 = (1 << 24) - 1; // 0xFFFFFF

#[derive(Debug, Clone)]
pub(crate) struct CMap {
    codespace_ranges: [Vec<u32>; 4],
    map: HashMap<u32, CMapValue>,
    name: String,
    vertical: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CMapValue {
    Cid(u32),
    BfString(String),
}

impl CMap {
    pub(crate) fn new() -> Self {
        CMap {
            codespace_ranges: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            map: HashMap::new(),
            name: String::new(),
            vertical: false,
        }
    }

    pub(crate) fn identity_h() -> Self {
        let mut cmap = CMap::new();
        
        cmap.name = "Identity-H".to_string();
        cmap.vertical = false;
        cmap.add_codespace_range(2, 0, 0xFFFF);
        cmap
    }

    pub(crate) fn identity_v() -> Self {
        let mut cmap = CMap::new();
        
        cmap.name = "Identity-V".to_string();
        cmap.vertical = true;
        cmap.add_codespace_range(2, 0, 0xFFFF);
        cmap
    }
    
    pub(crate) fn is_vertical(&self) -> bool {
        self.vertical
    }

    pub(crate) fn lookup_code(&self, code: u32) -> Option<CMapValue> {
        if let Some(value) = self.map.get(&code) {
            Some(value.clone())
        } else if self.is_identity_cmap() {
            if code <= 0xFFFF {
                Some(CMapValue::Cid(code))
            } else {
                None
            }
        } else {
            None
        }
    }

    fn add_codespace_range(&mut self, n: usize, low: u32, high: u32) {
        if n > 0 && n <= 4 {
            self.codespace_ranges[n - 1].push(low);
            self.codespace_ranges[n - 1].push(high);
        }
    }

    fn map_cid_range(&mut self, low: u32, high: u32, dst_low: u32) -> Option<()> {
        if high - low > MAX_MAP_RANGE {
            return None;
        }

        let mut current_low = low;
        let mut current_dst = dst_low;
        while current_low <= high {
            self.map.insert(current_low, CMapValue::Cid(current_dst));
            current_low += 1;
            current_dst += 1;
        }
        
        Some(())
    }

    fn map_bf_range(&mut self, low: u32, high: u32, dst_low: String) -> Option<()> {
        if high - low > MAX_MAP_RANGE {
            return None;
        }

        let mut current_low = low;
        let mut current_dst = dst_low;

        while current_low <= high {
            self.map
                .insert(current_low, CMapValue::BfString(current_dst.clone()));

            let mut bytes = current_dst.into_bytes();
            if let Some(last_byte) = bytes.last_mut() {
                if *last_byte == 0xff {
                    if bytes.len() > 1 {
                        let len = bytes.len();
                        bytes[len - 2] += 1;
                        bytes[len - 1] = 0x00;
                    }
                } else {
                    *last_byte += 1;
                }
            }
            current_dst = String::from_utf8_lossy(&bytes).to_string();
            current_low += 1;
        }
        
        Some(())
    }

    fn map_bf_range_to_array(
        &mut self,
        low: u32,
        high: u32,
        array: Vec<CMapValue>,
    ) -> Option<()> {
        if high - low > MAX_MAP_RANGE {
            return None;
        }

        let mut current_low = low;
        let mut i = 0;

        while current_low <= high && i < array.len() {
            self.map.insert(current_low, array[i].clone());
            current_low += 1;
            i += 1;
        }
        
        Some(())
    }

    fn map_one(&mut self, src: u32, dst: CMapValue) {
        self.map.insert(src, dst);
    }

    fn is_identity_cmap(&self) -> bool {
        (self.name == "Identity-H" || self.name == "Identity-V") && self.map.is_empty()
    }

    pub fn read_code(&self, bytes: &[u8], offset: usize) -> (u32, usize) {
        let mut c = 0u32;

        for n in 0..4.min(bytes.len() - offset) {
            if offset + n >= bytes.len() {
                break;
            }

            c = (c << 8) | bytes[offset + n] as u32;

            let codespace_range = &self.codespace_ranges[n];
            for chunk in codespace_range.chunks(2) {
                if chunk.len() == 2 {
                    let low = chunk[0];
                    let high = chunk[1];
                    if c >= low && c <= high {
                        return (c, n + 1);
                    }
                }
            }
        }

        (0, 1)
    }
}

fn str_to_int(s: &str) -> u32 {
    let mut a = 0u32;
    for ch in s.chars() {
        // Since we created these strings from bytes using char::from(byte),
        // we can safely cast back to get the original byte value
        a = (a << 8) | (ch as u32 & 0xFF);
    }
    a & 0xFFFFFFFF
}

fn expect_string(obj: &Token) -> Option<String> {
    match obj {
        Token::HexString(bytes) => {
            // Convert bytes to string the same way pdf.js does: using String.fromCharCode
            // Each byte becomes a character with that character code
            let mut result = String::new();
            for &byte in bytes {
                result.push(char::from(byte));
            }
            Some(result)
        }
        Token::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn expect_int(obj: &Token) -> Option<i32> {
    match obj {
        Token::Integer(i) => Some(*i),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub enum Token {
    String(String),
    HexString(Vec<u8>), // Raw bytes from hex string
    Integer(i32),
    Command(String),
    Name(String),
    EOF,
}

pub struct CMapLexer {
    input: String,
    position: usize,
}

impl CMapLexer {
    pub fn new(input: String) -> Self {
        CMapLexer { input, position: 0 }
    }

    pub fn get_obj(&mut self) -> Token {
        self.skip_whitespace();

        if self.position >= self.input.len() {
            return Token::EOF;
        }

        let remaining = &self.input[self.position..];

        // Handle PostScript comments (% to end of line)
        if remaining.starts_with('%') {
            // Skip to end of line
            while self.position < self.input.len() {
                let ch = self.input.chars().nth(self.position).unwrap();
                self.position += 1;
                if ch == '\n' || ch == '\r' {
                    break;
                }
            }
            // Skip any additional whitespace and try again
            self.skip_whitespace();
            return self.get_obj();
        }

        // Handle dictionary delimiters
        if remaining.starts_with(">>") {
            self.position += 2;
            return Token::Command(">>".to_string());
        }

        // Handle hex strings and dictionary start
        if remaining.starts_with('<') {
            return self.parse_hex_string();
        }

        // Handle PostScript strings (parentheses)
        if remaining.starts_with('(') {
            return self.parse_ps_string();
        }

        // Handle arrays
        if remaining.starts_with('[') {
            return self.parse_array();
        }

        if remaining.starts_with(']') {
            self.position += 1;
            return Token::Command("]".to_string());
        }

        // Handle names
        if remaining.starts_with('/') {
            return self.parse_name();
        }

        // Handle numbers and commands
        self.parse_token()
    }

    fn skip_whitespace(&mut self) {
        while self.position < self.input.len() {
            let ch = self.input.chars().nth(self.position).unwrap();
            if ch.is_whitespace() {
                self.position += 1;
            } else {
                break;
            }
        }
    }

    fn parse_hex_string(&mut self) -> Token {
        // Check if it's actually a dictionary delimiter <<
        let remaining = &self.input[self.position..];
        if remaining.starts_with("<<") {
            self.position += 2;
            return Token::Command("<<".to_string());
        }

        self.position += 1; // Skip '<'
        let mut hex_string = String::new();

        while self.position < self.input.len() {
            let ch = self.input.chars().nth(self.position).unwrap();
            if ch == '>' {
                self.position += 1;
                break;
            }
            if ch.is_ascii_hexdigit() {
                hex_string.push(ch);
            }
            self.position += 1;
        }

        // println!("Parsed hex string: {}", hex_string); // Debug

        // Convert hex string to raw bytes
        let mut result_bytes = Vec::new();
        for chunk in hex_string.chars().collect::<Vec<_>>().chunks(2) {
            let hex_byte = if chunk.len() == 2 {
                format!("{}{}", chunk[0], chunk[1])
            } else {
                format!("{}0", chunk[0])
            };

            if let Ok(byte_val) = u8::from_str_radix(&hex_byte, 16) {
                result_bytes.push(byte_val);
            }
        }

        // println!("Converted hex bytes: {:?}", result_bytes); // Debug
        Token::HexString(result_bytes)
    }

    fn parse_ps_string(&mut self) -> Token {
        self.position += 1; // Skip '('
        let mut string = String::new();
        let mut paren_depth = 1;

        while self.position < self.input.len() && paren_depth > 0 {
            let ch = self.input.chars().nth(self.position).unwrap();
            match ch {
                '(' => {
                    paren_depth += 1;
                    string.push(ch);
                }
                ')' => {
                    paren_depth -= 1;
                    if paren_depth > 0 {
                        string.push(ch);
                    }
                }
                '\\' => {
                    // Handle escape sequences
                    self.position += 1;
                    if self.position < self.input.len() {
                        let escaped = self.input.chars().nth(self.position).unwrap();
                        string.push('\\');
                        string.push(escaped);
                    }
                }
                _ => string.push(ch),
            }
            self.position += 1;
        }

        Token::String(string)
    }

    fn parse_array(&mut self) -> Token {
        self.position += 1; // Skip '['
        Token::Command("[".to_string())
    }

    fn parse_name(&mut self) -> Token {
        self.position += 1; // Skip '/'
        let mut name = String::new();

        while self.position < self.input.len() {
            let ch = self.input.chars().nth(self.position).unwrap();
            if ch.is_whitespace() || "[]<>(){}/%".contains(ch) {
                break;
            }
            name.push(ch);
            self.position += 1;
        }

        Token::Name(name)
    }

    fn parse_token(&mut self) -> Token {
        let mut token = String::new();

        while self.position < self.input.len() {
            let ch = self.input.chars().nth(self.position).unwrap();
            if ch.is_whitespace() || "[]<>(){}/%".contains(ch) {
                break;
            }
            token.push(ch);
            self.position += 1;
        }

        if token.is_empty() {
            return Token::EOF;
        }

        if let Ok(num) = token.parse::<i32>() {
            Token::Integer(num)
        } else {
            Token::Command(token)
        }
    }
}

fn parse_bf_char(cmap: &mut CMap, lexer: &mut CMapLexer) -> Option<()> {
    loop {
        let obj = lexer.get_obj();
        match obj {
            Token::EOF => break,
            Token::Command(cmd) if cmd == "endbfchar" => return Some(()),
            ref token => {
                let src_str = expect_string(token)?;
                let src = str_to_int(&src_str);
                let dst_obj = lexer.get_obj();
                let dst_str = expect_string(&dst_obj)?;
                // For beginbfchar, if the destination is a short hex string (like <0003>),
                // it represents a Unicode code point, not a multi-byte string
                if dst_str.chars().count() <= 2 {
                    // Convert to Unicode code point
                    let code_point = str_to_int(&dst_str);
                    if let Some(unicode_char) = char::from_u32(code_point) {
                        cmap.map_one(src, CMapValue::BfString(unicode_char.to_string()));
                    } else {
                        // Fallback to original string if invalid Unicode
                        cmap.map_one(src, CMapValue::BfString(dst_str));
                    }
                } else {
                    // For longer strings, keep as multi-byte string
                    cmap.map_one(src, CMapValue::BfString(dst_str));
                }
            }
        }
    }
    
    Some(())
}

fn parse_bf_range(cmap: &mut CMap, lexer: &mut CMapLexer) -> Option<()> {
    loop {
        let obj = lexer.get_obj();
        match obj {
            Token::EOF => break,
            Token::Command(cmd) if cmd == "endbfrange" => return Some(()),
            ref token => {
                let low_str = expect_string(token)?;
                let low = str_to_int(&low_str);

                let high_obj = lexer.get_obj();
                let high_str = expect_string(&high_obj)?;
                let high = str_to_int(&high_str);

                let dst_obj = lexer.get_obj();
                match dst_obj {
                    Token::Integer(dst_int) => {
                        let dst_low = char::from(dst_int as u8).to_string();
                        cmap.map_bf_range(low, high, dst_low)?;
                    }
                    ref token => {
                        if let Some(dst_str) = expect_string(token) {
                            cmap.map_bf_range(low, high, dst_str)?;
                        } else if let Token::Command(cmd) = token {
                            if cmd == "[" {
                                let mut array = Vec::new();
                                loop {
                                    let array_obj = lexer.get_obj();
                                    match array_obj {
                                        Token::Command(cmd) if cmd == "]" => break,
                                        Token::EOF => break,
                                        Token::Integer(val) => {
                                            array.push(CMapValue::Cid(val as u32))
                                        }
                                        ref arr_token => {
                                            if let Some(val_str) = expect_string(arr_token) {
                                                array.push(CMapValue::BfString(val_str));
                                            }
                                        }
                                    }
                                }
                                cmap.map_bf_range_to_array(low, high, array)?;
                            } else {
                                return None;
                            }
                        } else {
                            return None;
                        }
                    }
                }
            }
        }
    }
    
    Some(())
}

fn parse_cid_char(cmap: &mut CMap, lexer: &mut CMapLexer) -> Option<()> {
    loop {
        let obj = lexer.get_obj();
        match obj {
            Token::EOF => break,
            Token::Command(cmd) if cmd == "endcidchar" => return Some(()),
            ref token => {
                let src_str = expect_string(token)?;
                let src = str_to_int(&src_str);
                let dst_obj = lexer.get_obj();
                let dst = expect_int(&dst_obj)?;
                cmap.map_one(src, CMapValue::Cid(dst as u32));
            }
        }
    }
    
    Some(())
}

fn parse_cid_range(cmap: &mut CMap, lexer: &mut CMapLexer) -> Option<()> {
    loop {
        let obj = lexer.get_obj();
        match obj {
            Token::EOF => break,
            Token::Command(cmd) if cmd == "endcidrange" => return Some(()),
            ref token => {
                let low_str = expect_string(token)?;
                let low = str_to_int(&low_str);

                let high_obj = lexer.get_obj();
                let high_str = expect_string(&high_obj)?;
                let high = str_to_int(&high_str);

                let dst_obj = lexer.get_obj();
                let dst_low = expect_int(&dst_obj)?;

                cmap.map_cid_range(low, high, dst_low as u32)?;
            }
        }
    }
    
    Some(())
}

fn parse_codespace_range(cmap: &mut CMap, lexer: &mut CMapLexer) -> Option<()> {
    loop {
        let obj = lexer.get_obj();
        match obj {
            Token::EOF => break,
            Token::Command(cmd) if cmd == "endcodespacerange" => return Some(()),
            ref token => {
                let low_str = expect_string(token)?;
                if low_str.is_empty() {
                    continue;
                }
                let low = str_to_int(&low_str);

                let high_obj = lexer.get_obj();
                let high_str = expect_string(&high_obj)?;
                if high_str.is_empty() {
                    return None;
                }
                let high = str_to_int(&high_str);

                cmap.add_codespace_range(high_str.chars().count(), low, high);
            }
        }
    }
    
    Some(())
}

fn parse_wmode(cmap: &mut CMap, lexer: &mut CMapLexer) -> Option<()> {
    let obj = lexer.get_obj();
    if let Some(val) = expect_int(&obj) {
        cmap.vertical = val != 0;
    }
    
    Some(())
}

fn parse_cmap_name(cmap: &mut CMap, lexer: &mut CMapLexer) -> Option<()> {
    let obj = lexer.get_obj();
    match obj {
        Token::Name(name) => {
            cmap.name = name;
            Some(())
        }
        _ => Some(()), // Don't error on unexpected tokens, just ignore
    }
}

pub fn parse_cmap(input: String) -> Option<CMap> {
    let mut cmap = CMap::new();
    let mut lexer = CMapLexer::new(input);

    loop {
        let obj = lexer.get_obj();
        match obj {
            Token::EOF => break,
            Token::Name(ref name) => {
                if name == "WMode" {
                    parse_wmode(&mut cmap, &mut lexer)?;
                } else if name == "CMapName" {
                    parse_cmap_name(&mut cmap, &mut lexer)?;
                }
            }
            Token::Command(ref cmd) => {
                match cmd.as_str() {
                    "endcmap" => break,
                    "usecmap" => {
                        // TODO: Implement
                    }
                    "begincodespacerange" => {
                        parse_codespace_range(&mut cmap, &mut lexer)?;
                    }
                    "beginbfchar" => {
                        parse_bf_char(&mut cmap, &mut lexer)?;
                    }
                    "begincidchar" => {
                        parse_cid_char(&mut cmap, &mut lexer)?;
                    }
                    "beginbfrange" => {
                        parse_bf_range(&mut cmap, &mut lexer)?;
                    }
                    "begincidrange" => {
                        parse_cid_range(&mut cmap, &mut lexer)?;
                    }
                    // Skip PostScript constructs and other unknown commands.
                    "def" | "dict" | "begin" | "end" | "findresource" | "<<" | ">>" | "pop"
                    | "currentdict" | "defineresource" => {
                        // These are PostScript constructs that we can ignore.
                    }
                    _ => {
                        // Skip any other unknown commands.
                    }
                }
            }
            Token::String(_) | Token::HexString(_) | Token::Integer(_) => {
                // Skip standalone tokens that aren't part of a command we recognize.
            }
        }
    }

    Some(cmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_beginbfchar() {
        let input = r#"2 beginbfchar
<03> <00>
<04> <01>
endbfchar"#
            .to_string();

        let cmap = parse_cmap(input).unwrap();

        let Some(CMapValue::BfString(val)) = cmap.lookup_code(0x03) else {
            panic!("Expected BfString value for 0x03");
        };
        assert_eq!(val.chars().next().unwrap() as u32, 0x00);

        let Some(CMapValue::BfString(val)) = cmap.lookup_code(0x04) else {
            panic!("Expected BfString value for 0x04");
        };
        assert_eq!(val.chars().next().unwrap() as u32, 0x01);

        assert!(cmap.lookup_code(0x05).is_none());
    }

    #[test]
    fn test_parse_beginbfrange_with_range() {
        let input = r#"1 beginbfrange
<06> <0B> 0
endbfrange"#
            .to_string();

        let cmap = parse_cmap(input).unwrap();

        assert!(cmap.lookup_code(0x05).is_none());

        if let Some(CMapValue::BfString(val)) = cmap.lookup_code(0x06) {
            assert_eq!(val.chars().next().unwrap() as u32, 0x00);
        } else {
            panic!("Expected BfString value for 0x06");
        }

        if let Some(CMapValue::BfString(val)) = cmap.lookup_code(0x0b) {
            assert_eq!(val.chars().next().unwrap() as u32, 0x05);
        } else {
            panic!("Expected BfString value for 0x0b");
        }

        assert!(cmap.lookup_code(0x0c).is_none());
    }

    #[test]
    fn test_parse_beginbfrange_with_array() {
        let input = r#"1 beginbfrange
<0D> <12> [ 0 1 2 3 4 5 ]
endbfrange"#
            .to_string();

        let cmap = parse_cmap(input).unwrap();

        assert!(cmap.lookup_code(0x0c).is_none());

        if let Some(CMapValue::Cid(val)) = cmap.lookup_code(0x0d) {
            assert_eq!(val, 0x00);
        } else {
            panic!("Expected Cid value for 0x0d");
        }

        if let Some(CMapValue::Cid(val)) = cmap.lookup_code(0x12) {
            assert_eq!(val, 0x05);
        } else {
            panic!("Expected Cid value for 0x12");
        }

        assert!(cmap.lookup_code(0x13).is_none());
    }

    #[test]
    fn test_parse_begincidchar() {
        let input = r#"1 begincidchar
<14> 0
endcidchar"#
            .to_string();

        let cmap = parse_cmap(input).unwrap();

        if let Some(CMapValue::Cid(val)) = cmap.lookup_code(0x14) {
            assert_eq!(val, 0x00);
        } else {
            panic!("Expected Cid value for 0x14");
        }

        assert!(cmap.lookup_code(0x15).is_none());
    }

    #[test]
    fn test_parse_begincidrange() {
        let input = r#"1 begincidrange
<0016> <001B> 0
endcidrange"#
            .to_string();

        let cmap = parse_cmap(input).unwrap();

        assert!(cmap.lookup_code(0x15).is_none());

        if let Some(CMapValue::Cid(val)) = cmap.lookup_code(0x16) {
            assert_eq!(val, 0x00);
        } else {
            panic!("Expected Cid value for 0x16");
        }

        if let Some(CMapValue::Cid(val)) = cmap.lookup_code(0x1b) {
            assert_eq!(val, 0x05);
        } else {
            panic!("Expected Cid value for 0x1b");
        }

        assert!(cmap.lookup_code(0x1c).is_none());
    }

    #[test]
    fn test_parse_4_byte_codespace_ranges() {
        let input = r#"1 begincodespacerange
<8EA1A1A1> <8EA1FEFE>
endcodespacerange"#
            .to_string();

        let cmap = parse_cmap(input).unwrap();

        // Debug: Check if codespace ranges were parsed correctly
        println!("Codespace ranges: {:?}", cmap.codespace_ranges);

        // Use the new read_char_code_bytes method to handle raw bytes
        let test_bytes = [0x8E, 0xA1, 0xA1, 0xA1];
        let (charcode, length) = cmap.read_code(&test_bytes, 0);
        println!("Got charcode: {:#x}, length: {}", charcode, length);
        assert_eq!(charcode, 0x8ea1a1a1);
        assert_eq!(length, 4);
    }

    #[test]
    fn test_parse_cmap_name() {
        let input = r#"/CMapName /Identity-H def"#.to_string();

        let cmap = parse_cmap(input).unwrap();
        assert_eq!(cmap.name, "Identity-H");
    }

    #[test]
    fn test_parse_wmode() {
        let input = r#"/WMode 1 def"#.to_string();

        let cmap = parse_cmap(input).unwrap();
        assert_eq!(cmap.vertical, true);
    }

    #[test]
    fn test_identity_h_cmap() {
        let cmap = CMap::identity_h();

        assert_eq!(cmap.name, "Identity-H");
        assert_eq!(cmap.vertical, false);

        // Test identity mapping
        assert_eq!(cmap.lookup_code(0x41), Some(CMapValue::Cid(0x41))); // 'A'
        assert_eq!(cmap.lookup_code(0x1234), Some(CMapValue::Cid(0x1234)));
        assert_eq!(cmap.lookup_code(0xFFFF), Some(CMapValue::Cid(0xFFFF)));
        assert_eq!(cmap.lookup_code(0x10000), None); // Beyond 2-byte range

        // Test read_char_code with 2-byte values
        let test_bytes = [0x12, 0x34];
        let (charcode, length) = cmap.read_code(&test_bytes, 0);
        assert_eq!(charcode, 0x1234);
        assert_eq!(length, 2);
    }

    #[test]
    fn test_identity_v_cmap() {
        let cmap = CMap::identity_v();

        assert_eq!(cmap.name, "Identity-V");
        assert_eq!(cmap.vertical, true);

        // Test identity mapping
        assert_eq!(cmap.lookup_code(0x41), Some(CMapValue::Cid(0x41))); // 'A'
        assert_eq!(cmap.lookup_code(0x1234), Some(CMapValue::Cid(0x1234)));
        assert_eq!(cmap.lookup_code(0xFFFF), Some(CMapValue::Cid(0xFFFF)));
        assert_eq!(cmap.lookup_code(0x10000), None); // Beyond 2-byte range
    }

    #[test]
    fn test_simple_cidrange() {
        let input = r#"1 begincidrange
<00> <FF> 0
endcidrange"#
            .to_string();

        let cmap = parse_cmap(input).unwrap();

        // Should map codes 0x00-0xFF to CIDs 0-255
        assert_eq!(cmap.lookup_code(0x00), Some(CMapValue::Cid(0)));
        assert_eq!(cmap.lookup_code(0x41), Some(CMapValue::Cid(65))); // 'A'
        assert_eq!(cmap.lookup_code(0xFF), Some(CMapValue::Cid(255)));
        assert_eq!(cmap.lookup_code(0x100), None); // Beyond range
    }

    #[test]
    fn test_complex_cmap_with_postscript() {
        let input = r#"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo
<< /Registry (Adobe)
/Ordering (Identity)
/Supplement 0
>> def
/CMapName /Identity-H def
/CMapType 2 def
1 begincodespacerange
<00> <FF>
endcodespacerange
1 begincidrange
<00> <FF> 0
endcidrange
endcmap
CMapName currentdict /CMap defineresource pop
end
end"#
            .to_string();

        let cmap = parse_cmap(input).unwrap();

        // Debug: print the cmap state
        // println!("CMap name: {}", cmap.name);
        // println!("CMap num_codespace_ranges: {}", cmap.num_codespace_ranges);
        // println!("CMap map length: {}", cmap.map.len());

        // Should successfully parse and create mapping
        assert_eq!(cmap.lookup_code(0x00), Some(CMapValue::Cid(0)));
        assert_eq!(cmap.lookup_code(0x41), Some(CMapValue::Cid(65))); // 'A'
        assert_eq!(cmap.lookup_code(0xFF), Some(CMapValue::Cid(255)));
        assert_eq!(cmap.lookup_code(0x100), None); // Beyond range
        assert_eq!(cmap.name, "Identity-H");
    }

    #[test]
    fn another_cmap() {
        let loaded = std::str::from_utf8(
            &std::fs::read(
                "/Users/lstampfl/Programming/GitHub/hayro/hayro-interpret/src/font/cmap.txt",
            )
            .unwrap(),
        )
        .unwrap()
        .to_string();
        let cmap = parse_cmap(loaded).unwrap();

        // Verify the CMAP was parsed correctly
        assert_eq!(cmap.name, "CM10");
        assert!(!cmap.vertical);
        assert_eq!(cmap.codespace_ranges[1], vec![0, 65535]); // 2-byte range 0x0000-0xFFFF
        assert!(!cmap.map.is_empty()); // Should have character mappings

        // Test a few specific mappings from the file
        // The file maps: <0020> <0003> and <0021> <0004>
        // These should be Unicode characters U+0003 and U+0004
        assert_eq!(
            cmap.lookup_code(0x20),
            Some(CMapValue::BfString("\u{3}".to_string()))
        ); // space maps to U+0003
        assert_eq!(
            cmap.lookup_code(0x21),
            Some(CMapValue::BfString("\u{4}".to_string()))
        ); // exclamation maps to U+0004
    }
}
