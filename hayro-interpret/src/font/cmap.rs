use std::collections::HashMap;

const MAX_MAP_RANGE: u32 = (1 << 24) - 1; // 0xFFFFFF

#[derive(Debug, Clone)]
pub struct CMap {
    pub codespace_ranges: [Vec<u32>; 4],
    pub num_codespace_ranges: usize,
    map: HashMap<u32, CMapValue>,
    pub name: String,
    pub vertical: bool,
    pub use_cmap: Option<Box<CMap>>,
    pub built_in_cmap: bool,
}

#[derive(Debug, Clone)]
pub enum CMapValue {
    Cid(u32),
    BfString(String),
}

impl CMap {
    pub fn new(built_in_cmap: bool) -> Self {
        CMap {
            codespace_ranges: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            num_codespace_ranges: 0,
            map: HashMap::new(),
            name: String::new(),
            vertical: false,
            use_cmap: None,
            built_in_cmap,
        }
    }

    pub fn add_codespace_range(&mut self, n: usize, low: u32, high: u32) {
        if n > 0 && n <= 4 {
            self.codespace_ranges[n - 1].push(low);
            self.codespace_ranges[n - 1].push(high);
            self.num_codespace_ranges += 1;
        }
    }

    pub fn map_cid_range(&mut self, low: u32, high: u32, dst_low: u32) -> Result<(), String> {
        if high - low > MAX_MAP_RANGE {
            return Err("mapCidRange - ignoring data above MAX_MAP_RANGE.".to_string());
        }
        
        let mut current_low = low;
        let mut current_dst = dst_low;
        while current_low <= high {
            self.map.insert(current_low, CMapValue::Cid(current_dst));
            current_low += 1;
            current_dst += 1;
        }
        Ok(())
    }

    pub fn map_bf_range(&mut self, low: u32, high: u32, dst_low: String) -> Result<(), String> {
        if high - low > MAX_MAP_RANGE {
            return Err("mapBfRange - ignoring data above MAX_MAP_RANGE.".to_string());
        }

        let mut current_low = low;
        let mut current_dst = dst_low;
        
        while current_low <= high {
            self.map.insert(current_low, CMapValue::BfString(current_dst.clone()));
            
            // Increment the last byte of the string
            let mut bytes = current_dst.into_bytes();
            if let Some(last_byte) = bytes.last_mut() {
                if *last_byte == 0xff {
                    // Handle overflow by incrementing the previous byte
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
        Ok(())
    }

    pub fn map_bf_range_to_array(&mut self, low: u32, high: u32, array: Vec<CMapValue>) -> Result<(), String> {
        if high - low > MAX_MAP_RANGE {
            return Err("mapBfRangeToArray - ignoring data above MAX_MAP_RANGE.".to_string());
        }

        let mut current_low = low;
        let mut i = 0;
        
        while current_low <= high && i < array.len() {
            self.map.insert(current_low, array[i].clone());
            current_low += 1;
            i += 1;
        }
        Ok(())
    }

    pub fn map_one(&mut self, src: u32, dst: CMapValue) {
        self.map.insert(src, dst);
    }

    pub fn lookup(&self, code: u32) -> Option<&CMapValue> {
        if let Some(value) = self.map.get(&code) {
            Some(value)
        } else if let Some(ref use_cmap) = self.use_cmap {
            use_cmap.lookup(code)
        } else {
            None
        }
    }

    pub fn contains(&self, code: u32) -> bool {
        self.map.contains_key(&code) || 
        self.use_cmap.as_ref().map_or(false, |use_cmap| use_cmap.contains(code))
    }

    pub fn read_char_code(&self, s: &str, offset: usize) -> (u32, usize) {
        let mut c = 0u32;
        let bytes = s.as_bytes();
        
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

    pub fn read_char_code_bytes(&self, bytes: &[u8], offset: usize) -> (u32, usize) {
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

    pub fn get_char_code_length(&self, char_code: u32) -> usize {
        for n in 0..4 {
            let codespace_range = &self.codespace_ranges[n];
            for chunk in codespace_range.chunks(2) {
                if chunk.len() == 2 {
                    let low = chunk[0];
                    let high = chunk[1];
                    if char_code >= low && char_code <= high {
                        return n + 1;
                    }
                }
            }
        }
        1
    }

    pub fn length(&self) -> usize {
        self.map.len()
    }
}

fn str_to_int(s: &str) -> u32 {
    let mut a = 0u32;
    for byte in s.bytes() {
        a = (a << 8) | byte as u32;
    }
    a
}

fn bytes_to_int(bytes: &[u8]) -> u32 {
    let mut a = 0u32;
    for &byte in bytes {
        a = (a << 8) | byte as u32;
    }
    a
}

fn expect_string(_token: &str) -> Result<(), String> {
    // In this simplified implementation, we assume all tokens are valid strings
    Ok(())
}

fn expect_int(_value: i32) -> Result<(), String> {
    // Simple validation that the value is a valid integer
    Ok(())
}

#[derive(Debug, Clone)]
pub enum Token {
    String(String),
    HexString(Vec<u8>), // Raw bytes from hex string
    Integer(i32),
    Command(String),
    Name(String),
    Array(Vec<Token>),
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
        
        // Handle hex strings
        if remaining.starts_with('<') {
            return self.parse_hex_string();
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
        
        // Try to parse as integer
        if let Ok(num) = token.parse::<i32>() {
            Token::Integer(num)
        } else {
            Token::Command(token)
        }
    }
}

pub fn parse_bf_char(cmap: &mut CMap, lexer: &mut CMapLexer) -> Result<(), String> {
    loop {
        let obj = lexer.get_obj();
        match obj {
            Token::EOF => break,
            Token::Command(cmd) if cmd == "endbfchar" => return Ok(()),
            Token::HexString(src_bytes) => {
                let src = bytes_to_int(&src_bytes);
                let dst_obj = lexer.get_obj();
                match dst_obj {
                    Token::HexString(dst_bytes) => {
                        let dst_string = String::from_utf8_lossy(&dst_bytes).to_string();
                        cmap.map_one(src, CMapValue::BfString(dst_string));
                    }
                    _ => return Err("Expected string after bf char source".to_string()),
                }
            }
            _ => return Err("Expected string in bf char".to_string()),
        }
    }
    Ok(())
}

pub fn parse_bf_range(cmap: &mut CMap, lexer: &mut CMapLexer) -> Result<(), String> {
    loop {
        let obj = lexer.get_obj();
        match obj {
            Token::EOF => break,
            Token::Command(cmd) if cmd == "endbfrange" => return Ok(()),
            Token::HexString(low_bytes) => {
                let low = bytes_to_int(&low_bytes);
                
                let high_obj = lexer.get_obj();
                let high = match high_obj {
                    Token::HexString(high_bytes) => bytes_to_int(&high_bytes),
                    _ => return Err("Expected string for high value in bf range".to_string()),
                };
                
                let dst_obj = lexer.get_obj();
                match dst_obj {
                    Token::Integer(dst_int) => {
                        let dst_low = String::from_utf8(vec![dst_int as u8]).unwrap_or_default();
                        cmap.map_bf_range(low, high, dst_low)?;
                    }
                    Token::HexString(dst_bytes) => {
                        let dst_string = String::from_utf8_lossy(&dst_bytes).to_string();
                        cmap.map_bf_range(low, high, dst_string)?;
                    }
                    Token::Command(cmd) if cmd == "[" => {
                        let mut array = Vec::new();
                        loop {
                            let array_obj = lexer.get_obj();
                            match array_obj {
                                Token::Command(cmd) if cmd == "]" => break,
                                Token::EOF => break,
                                Token::Integer(val) => array.push(CMapValue::Cid(val as u32)),
                                Token::HexString(val_bytes) => {
                                    let val_string = String::from_utf8_lossy(&val_bytes).to_string();
                                    array.push(CMapValue::BfString(val_string));
                                }
                                _ => {}
                            }
                        }
                        cmap.map_bf_range_to_array(low, high, array)?;
                    }
                    _ => return Err("Invalid bf range destination".to_string()),
                }
            }
            _ => return Err("Expected string in bf range".to_string()),
        }
    }
    Ok(())
}

pub fn parse_cid_char(cmap: &mut CMap, lexer: &mut CMapLexer) -> Result<(), String> {
    loop {
        let obj = lexer.get_obj();
        match obj {
            Token::EOF => break,
            Token::Command(cmd) if cmd == "endcidchar" => return Ok(()),
            Token::HexString(src_bytes) => {
                let src = bytes_to_int(&src_bytes);
                let dst_obj = lexer.get_obj();
                match dst_obj {
                    Token::Integer(dst) => {
                        cmap.map_one(src, CMapValue::Cid(dst as u32));
                    }
                    _ => return Err("Expected integer after cid char source".to_string()),
                }
            }
            _ => return Err("Expected string in cid char".to_string()),
        }
    }
    Ok(())
}

pub fn parse_cid_range(cmap: &mut CMap, lexer: &mut CMapLexer) -> Result<(), String> {
    loop {
        let obj = lexer.get_obj();
        match obj {
            Token::EOF => break,
            Token::Command(cmd) if cmd == "endcidrange" => return Ok(()),
            Token::HexString(low_bytes) => {
                let low = bytes_to_int(&low_bytes);
                
                let high_obj = lexer.get_obj();
                let high = match high_obj {
                    Token::HexString(high_bytes) => bytes_to_int(&high_bytes),
                    _ => return Err("Expected string for high value in cid range".to_string()),
                };
                
                let dst_obj = lexer.get_obj();
                match dst_obj {
                    Token::Integer(dst_low) => {
                        cmap.map_cid_range(low, high, dst_low as u32)?;
                    }
                    _ => return Err("Expected integer for destination in cid range".to_string()),
                }
            }
            _ => return Err("Expected string in cid range".to_string()),
        }
    }
    Ok(())
}

pub fn parse_codespace_range(cmap: &mut CMap, lexer: &mut CMapLexer) -> Result<(), String> {
    loop {
        let obj = lexer.get_obj();
        // println!("In parse_codespace_range, token: {:?}", obj);
        match obj {
            Token::EOF => break,
            Token::Command(cmd) if cmd == "endcodespacerange" => return Ok(()),
            Token::HexString(low_bytes) => {
                let low = bytes_to_int(&low_bytes);
                // println!("Low value: {}, bytes: {:?}, byte_len: {}", low, low_bytes, low_bytes.len());
                
                let high_obj = lexer.get_obj();
                // println!("High token: {:?}", high_obj);
                match high_obj {
                    Token::HexString(high_bytes) => {
                        let high = bytes_to_int(&high_bytes);
                        // println!("High value: {}, bytes: {:?}, byte_len: {}", high, high_bytes, high_bytes.len());
                        // println!("Adding codespace range: n={}, low={}, high={}", high_bytes.len(), low, high);
                        cmap.add_codespace_range(high_bytes.len(), low, high);
                    }
                    _ => return Err("Expected string for high value in codespace range".to_string()),
                }
            }
            _ => return Err("Expected string in codespace range".to_string()),
        }
    }
    Ok(())
}

pub fn parse_wmode(cmap: &mut CMap, lexer: &mut CMapLexer) -> Result<(), String> {
    let obj = lexer.get_obj();
    match obj {
        Token::Integer(val) => {
            cmap.vertical = val != 0;
            Ok(())
        }
        _ => Err("Expected integer for WMode".to_string()),
    }
}

pub fn parse_cmap_name(cmap: &mut CMap, lexer: &mut CMapLexer) -> Result<(), String> {
    let obj = lexer.get_obj();
    match obj {
        Token::Name(name) => {
            cmap.name = name;
            Ok(())
        }
        _ => Err("Expected name for CMapName".to_string()),
    }
}

pub fn parse_cmap(input: String) -> Result<CMap, String> {
    let mut cmap = CMap::new(false);
    let mut lexer = CMapLexer::new(input);

    loop {
        let obj = lexer.get_obj();
        // println!("Token: {:?}", obj);  // Debug line
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
                        // Handle usecmap - for now just skip
                    }
                    "begincodespacerange" => {
                        // println!("Found begincodespacerange");
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
                    _ => {}
                }
            }
            _ => {}
        }
    }

    Ok(cmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_beginbfchar() {
        let input = r#"2 beginbfchar
<03> <00>
<04> <01>
endbfchar"#.to_string();
        
        let cmap = parse_cmap(input).unwrap();
        
        if let Some(CMapValue::BfString(val)) = cmap.lookup(0x03) {
            assert_eq!(val.chars().next().unwrap() as u32, 0x00);
        } else {
            panic!("Expected BfString value for 0x03");
        }
        
        if let Some(CMapValue::BfString(val)) = cmap.lookup(0x04) {
            assert_eq!(val.chars().next().unwrap() as u32, 0x01);
        } else {
            panic!("Expected BfString value for 0x04");
        }
        
        assert!(cmap.lookup(0x05).is_none());
    }

    #[test]
    fn test_parse_beginbfrange_with_range() {
        let input = r#"1 beginbfrange
<06> <0B> 0
endbfrange"#.to_string();
        
        let cmap = parse_cmap(input).unwrap();
        
        assert!(cmap.lookup(0x05).is_none());
        
        if let Some(CMapValue::BfString(val)) = cmap.lookup(0x06) {
            assert_eq!(val.chars().next().unwrap() as u32, 0x00);
        } else {
            panic!("Expected BfString value for 0x06");
        }
        
        if let Some(CMapValue::BfString(val)) = cmap.lookup(0x0b) {
            assert_eq!(val.chars().next().unwrap() as u32, 0x05);
        } else {
            panic!("Expected BfString value for 0x0b");
        }
        
        assert!(cmap.lookup(0x0c).is_none());
    }

    #[test]
    fn test_parse_beginbfrange_with_array() {
        let input = r#"1 beginbfrange
<0D> <12> [ 0 1 2 3 4 5 ]
endbfrange"#.to_string();
        
        let cmap = parse_cmap(input).unwrap();
        
        assert!(cmap.lookup(0x0c).is_none());
        
        if let Some(CMapValue::Cid(val)) = cmap.lookup(0x0d) {
            assert_eq!(*val, 0x00);
        } else {
            panic!("Expected Cid value for 0x0d");
        }
        
        if let Some(CMapValue::Cid(val)) = cmap.lookup(0x12) {
            assert_eq!(*val, 0x05);
        } else {
            panic!("Expected Cid value for 0x12");
        }
        
        assert!(cmap.lookup(0x13).is_none());
    }

    #[test]
    fn test_parse_begincidchar() {
        let input = r#"1 begincidchar
<14> 0
endcidchar"#.to_string();
        
        let cmap = parse_cmap(input).unwrap();
        
        if let Some(CMapValue::Cid(val)) = cmap.lookup(0x14) {
            assert_eq!(*val, 0x00);
        } else {
            panic!("Expected Cid value for 0x14");
        }
        
        assert!(cmap.lookup(0x15).is_none());
    }

    #[test]
    fn test_parse_begincidrange() {
        let input = r#"1 begincidrange
<0016> <001B> 0
endcidrange"#.to_string();
        
        let cmap = parse_cmap(input).unwrap();
        
        assert!(cmap.lookup(0x15).is_none());
        
        if let Some(CMapValue::Cid(val)) = cmap.lookup(0x16) {
            assert_eq!(*val, 0x00);
        } else {
            panic!("Expected Cid value for 0x16");
        }
        
        if let Some(CMapValue::Cid(val)) = cmap.lookup(0x1b) {
            assert_eq!(*val, 0x05);
        } else {
            panic!("Expected Cid value for 0x1b");
        }
        
        assert!(cmap.lookup(0x1c).is_none());
    }

    #[test]
    fn test_parse_codespace_ranges() {
        let input = r#"1 begincodespacerange
<01> <02>
<00000003> <00000004>
endcodespacerange"#.to_string();
        
        let cmap = parse_cmap(input).unwrap();
        
        let (charcode, length) = cmap.read_char_code("\x01", 0);
        assert_eq!(charcode, 1);
        assert_eq!(length, 1);
        
        let (charcode, length) = cmap.read_char_code("\x00\x00\x00\x03", 0);
        assert_eq!(charcode, 3);
        assert_eq!(length, 4);
    }

    #[test]
    fn test_parse_4_byte_codespace_ranges() {
        let input = r#"1 begincodespacerange
<8EA1A1A1> <8EA1FEFE>
endcodespacerange"#.to_string();
        
        let cmap = parse_cmap(input).unwrap();
        
        // Debug: Check if codespace ranges were parsed correctly
        // println!("Codespace ranges: {:?}", cmap.codespace_ranges);
        // println!("Num codespace ranges: {}", cmap.num_codespace_ranges);
        
        // Use the new read_char_code_bytes method to handle raw bytes
        let test_bytes = [0x8E, 0xA1, 0xA1, 0xA1];
        let (charcode, length) = cmap.read_char_code_bytes(&test_bytes, 0);
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
}