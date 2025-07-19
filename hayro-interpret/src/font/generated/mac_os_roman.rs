// See <https://github.com/apache/pdfbox/blob/4438b8fdc67a3a9ebfb194595d0e81f88b708a37/pdfbox/src/main/java/org/apache/pdfbox/pdmodel/font/encoding/MacOSRomanEncoding.java.java>
use phf::phf_map;

pub(crate) fn get(code: u8) -> Option<&'static str> {
    MAC_OS_ROMAN.get(&code).copied()
}

pub(crate) fn get_inverse(name: &str) -> Option<u8> {
    MAC_OS_ROMAN_INVERSE.get(name).copied()
}

static MAC_OS_ROMAN: phf::Map<u8, &'static str> = phf_map! {
    173u8 => "notequal",
    176u8 => "infinity",
    178u8 => "lessequal",
    179u8 => "greaterequal",
    182u8 => "partialdiff",
    183u8 => "summation",
    184u8 => "product",
    185u8 => "pi",
    186u8 => "integral",
    189u8 => "Omega",
    195u8 => "radical",
    197u8 => "approxequal",
    198u8 => "Delta",
    215u8 => "lozenge",
    219u8 => "Euro",
    240u8 => "apple"
};

static MAC_OS_ROMAN_INVERSE: phf::Map<&'static str, u8> = phf_map! {
    "notequal" => 0o0255u8,
    "infinity" => 0o0260u8,
    "lessequal" => 0o0262u8,
    "greaterequal" => 0o0263u8,
    "partialdiff" => 0o0266u8,
    "summation" => 0o0267u8,
    "product" => 0o0270u8,
    "pi" => 0o0271u8,
    "integral" => 0o0272u8,
    "Omega" => 0o0275u8,
    "radical" => 0o0303u8,
    "approxequal" => 0o0305u8,
    "Delta" => 0o0306u8,
    "lozenge" => 0o0327u8,
    "Euro" => 0o0333u8,
    "apple" => 0o0360u8,
};
