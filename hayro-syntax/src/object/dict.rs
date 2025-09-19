//! Dictionaries.

use crate::object::macros::object;
use crate::object::r#ref::{MaybeRef, ObjRef};
use crate::object::{Name, ObjectIdentifier};
use crate::object::{Object, ObjectLike};
use crate::reader::{Readable, Reader, ReaderContext, Skippable};
use crate::xref::XRef;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::ops::Deref;
use std::sync::Arc;

/// A dictionary, which is a key-value map, keys being names, and values being any PDF object or
/// objetc reference.
#[derive(Clone)]
pub struct Dict<'a>(Arc<Repr<'a>>);

impl Default for Dict<'_> {
    fn default() -> Self {
        Self::empty()
    }
}

// Note that this is not structural equality, i.e. two dictionaries with the same
// items are still considered different if they have different whitespaces.
impl PartialEq for Dict<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.data == other.0.data
    }
}

impl<'a> Dict<'a> {
    /// Create a new empty dictionary.
    pub fn empty() -> Dict<'a> {
        let repr = Repr {
            data: &[],
            offsets: Default::default(),
            ctx: ReaderContext::new(XRef::dummy(), false),
        };

        Self(Arc::new(repr))
    }

    /// Get the raw bytes underlying to the dictionary.
    pub fn data(&self) -> &'a [u8] {
        self.0.data
    }

    /// Returns the number of entries in the dictionary.
    pub fn len(&self) -> usize {
        self.0.offsets.len()
    }

    /// Return whether the dictionary is empty.
    pub fn is_empty(&self) -> bool {
        self.0.offsets.is_empty()
    }

    /// Checks whether the dictionary contains an entry with a specific key.
    pub fn contains_key(&self, key: impl Deref<Target = [u8]>) -> bool {
        self.0
            .offsets
            .contains_key(&Name::from_unescaped(key.deref()))
    }

    /// Returns the entry of a key as a specific object, or try to resolve it in case it's
    /// an object reference.
    #[allow(
        private_bounds,
        reason = "users shouldn't be able to implement `ObjectLike` for custom objects."
    )]
    pub fn get<'b, T>(&self, key: impl Deref<Target = [u8]>) -> Option<T>
    where
        T: ObjectLike<'a>,
    {
        self.get_raw::<T>(key.as_ref())?.resolve(&self.0.ctx)
    }

    /// Get the object reference linked to a key.
    pub fn get_ref(&self, key: impl Deref<Target = [u8]>) -> Option<ObjRef> {
        let offset = *self.0.offsets.get(&Name::from_unescaped(key.as_ref()))?;

        Reader::new(&self.0.data[offset..]).read_with_context::<ObjRef>(&self.0.ctx)
    }

    /// Returns an iterator over all keys in the dictionary.
    pub fn keys(&self) -> impl Iterator<Item = Name<'a>> {
        self.0.offsets.keys().cloned()
    }

    /// An iterator over all entries in the dictionary, sorted by key.
    pub fn entries(&self) -> impl Iterator<Item = (Name<'a>, MaybeRef<Object<'a>>)> {
        let mut sorted_keys = self.keys().collect::<Vec<_>>();
        sorted_keys.sort_by(|n1, n2| n1.as_ref().cmp(n2.as_ref()));
        sorted_keys.into_iter().map(|k| {
            let obj = self.get_raw(k.deref()).unwrap();
            (k, obj)
        })
    }

    /// Return the object identifier of the dict, if it's an indirect object.
    pub fn obj_id(&self) -> Option<ObjectIdentifier> {
        self.0.ctx.obj_number
    }

    /// Return the raw entry for a specific key.
    #[allow(private_bounds)]
    pub fn get_raw<T>(&self, key: impl Deref<Target = [u8]>) -> Option<MaybeRef<T>>
    where
        T: Readable<'a>,
    {
        let offset = *self.0.offsets.get(&Name::from_unescaped(key.as_ref()))?;

        Reader::new(&self.0.data[offset..]).read_with_context::<MaybeRef<T>>(&self.0.ctx)
    }

    pub(crate) fn ctx(&self) -> &ReaderContext<'a> {
        &self.0.ctx
    }
}

impl Debug for Dict<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut r = Reader::new(self.0.data);
        let mut debug_struct = f.debug_struct("Dict");

        for (key, val) in &self.0.offsets {
            r.jump(*val);
            debug_struct.field(
                &format!("{:?}", key.as_str()),
                &r.read_with_context::<MaybeRef<Object>>(&ReaderContext::dummy())
                    .unwrap(),
            );
        }
        Ok(())
    }
}

impl Skippable for Dict<'_> {
    fn skip(r: &mut Reader<'_>, is_content_stream: bool) -> Option<()> {
        r.forward_tag(b"<<")?;

        loop {
            r.skip_white_spaces_and_comments();

            if let Some(()) = r.forward_tag(b">>") {
                break Some(());
            } else {
                let Some(_) = r.skip::<Name>(is_content_stream) else {
                    // In case there is garbage in-between, be lenient and just try to skip it.
                    r.skip::<Object>(is_content_stream)?;
                    continue;
                };

                r.skip_white_spaces_and_comments();

                if is_content_stream {
                    r.skip::<Object>(is_content_stream)?;
                } else {
                    r.skip::<MaybeRef<Object>>(is_content_stream)?;
                }
            }
        }
    }
}

impl<'a> Readable<'a> for Dict<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        read_inner(r, ctx, Some(b"<<"), b">>")
    }
}

fn read_inner<'a>(
    r: &mut Reader<'a>,
    ctx: &ReaderContext<'a>,
    start_tag: Option<&[u8]>,
    end_tag: &[u8],
) -> Option<Dict<'a>> {
    let mut offsets = HashMap::new();

    let data = {
        let dict_data = r.tail()?;
        let start_offset = r.offset();

        // Inline image dictionaries don't start with '<<'.
        if let Some(start_tag) = start_tag {
            r.forward_tag(start_tag)?;
        }

        loop {
            r.skip_white_spaces_and_comments();

            // Normal dictionaries end with '>>', inline image dictionaries end with BD.
            if let Some(()) = r.peek_tag(end_tag) {
                r.forward_tag(end_tag)?;
                let end_offset = r.offset() - start_offset;

                break &dict_data[..end_offset];
            } else {
                let Some(name) = r.read_without_context::<Name>() else {
                    if start_tag.is_some() {
                        // In case there is garbage in-between, be lenient and just try to skip it.
                        // But only do this if we are parsing a proper dictionary as opposed to an
                        // inline dictionary.
                        r.read::<Object>(ctx)?;
                        continue;
                    } else {
                        return None;
                    }
                };
                r.skip_white_spaces_and_comments();

                // Do note that we are including objects in our dictionary even if they
                // are the null object, meaning that a call to `contains_key` will return `true`
                // even if the object is the null object. The PDF reference in theory requires
                // us to treat them as non-existing. Previously, we included a check to determine
                // whether the object is `null` before inserting it, but that caused problems
                // in some test cases where encryption + object streams are involved (as we would
                // attempt to read an object stream before having resolved the encryption dictionary).
                let offset = r.offset() - start_offset;
                offsets.insert(name, offset);

                if ctx.in_content_stream {
                    r.skip::<Object>(ctx.in_content_stream)?;
                } else {
                    r.skip::<MaybeRef<Object>>(ctx.in_content_stream)?;
                }
            }
        }
    };

    Some(Dict(Arc::new(Repr {
        data,
        offsets,
        ctx: ctx.clone(),
    })))
}

object!(Dict<'a>, Dict);

struct Repr<'a> {
    data: &'a [u8],
    offsets: HashMap<Name<'a>, usize>,
    ctx: ReaderContext<'a>,
}

pub(crate) struct InlineImageDict<'a>(Dict<'a>);

impl<'a> InlineImageDict<'a> {
    pub(crate) fn get_dict(&self) -> &Dict<'a> {
        &self.0
    }
}

impl<'a> Readable<'a> for InlineImageDict<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        Some(Self(read_inner(r, ctx, None, b"ID")?))
    }
}

/// A collection of possible keys in a PDF dictionary. Copied and adapted from PDFBox.
#[allow(missing_docs)]
pub mod keys {
    macro_rules! key {
        ($i:ident, $e:expr) => {
            pub const $i: &'static [u8] = $e;
        };
    }

    // A
    key!(A, b"A");
    key!(AA, b"AA");
    key!(ABSOLUTE_COLORIMETRIC, b"AbsoluteColorimetric");
    key!(AC, b"AC");
    key!(ACRO_FORM, b"AcroForm");
    key!(ACTUAL_TEXT, b"ActualText");
    key!(ADBE, b"ADBE");
    key!(ADBE_PKCS7_DETACHED, b"adbe.pkcs7.detached");
    key!(ADBE_PKCS7_SHA1, b"adbe.pkcs7.sha1");
    key!(ADBE_X509_RSA_SHA1, b"adbe.x509.rsa_sha1");
    key!(ADOBE_PPKLITE, b"Adobe.PPKLite");
    key!(AESV2, b"AESV2");
    key!(AESV3, b"AESV3");
    key!(AF, b"AF");
    key!(AF_RELATIONSHIP, b"AFRelationship");
    key!(AFTER, b"After");
    key!(AI_META_DATA, b"AIMetaData");
    key!(AIS, b"AIS");
    key!(ALL_OFF, b"AllOff");
    key!(ALL_ON, b"AllOn");
    key!(ALT, b"Alt");
    key!(ALPHA, b"Alpha");
    key!(ALTERNATE, b"Alternate");
    key!(ANNOT, b"Annot");
    key!(ANNOTS, b"Annots");
    key!(ANTI_ALIAS, b"AntiAlias");
    key!(ANY_OFF, b"AnyOff");
    key!(ANY_ON, b"AnyOn");
    key!(AP, b"AP");
    key!(AP_REF, b"APRef");
    key!(APP, b"App");
    key!(ART_BOX, b"ArtBox");
    key!(ARTIFACT, b"Artifact");
    key!(AS, b"AS");
    key!(ASCENT, b"Ascent");
    key!(ASCII_HEX_DECODE, b"ASCIIHexDecode");
    key!(ASCII_HEX_DECODE_ABBREVIATION, b"AHx");
    key!(ASCII85_DECODE, b"ASCII85Decode");
    key!(ASCII85_DECODE_ABBREVIATION, b"A85");
    key!(ATTACHED, b"Attached");
    key!(AUTHOR, b"Author");
    key!(AVG_WIDTH, b"AvgWidth");

    // B
    key!(B, b"B");
    key!(BACKGROUND, b"Background");
    key!(BASE_ENCODING, b"BaseEncoding");
    key!(BASE_FONT, b"BaseFont");
    key!(BASE_STATE, b"BaseState");
    key!(BASE_VERSION, b"BaseVersion");
    key!(BBOX, b"BBox");
    key!(BC, b"BC");
    key!(BE, b"BE");
    key!(BEAD, b"BEAD");
    key!(BEFORE, b"Before");
    key!(BG, b"BG");
    key!(BITS_PER_COMPONENT, b"BitsPerComponent");
    key!(BITS_PER_COORDINATE, b"BitsPerCoordinate");
    key!(BITS_PER_FLAG, b"BitsPerFlag");
    key!(BITS_PER_SAMPLE, b"BitsPerSample");
    key!(BL, b"Bl");
    key!(BLACK_IS_1, b"BlackIs1");
    key!(BLACK_POINT, b"BlackPoint");
    key!(BLEED_BOX, b"BleedBox");
    key!(BM, b"BM");
    key!(BORDER, b"Border");
    key!(BOUNDS, b"Bounds");
    key!(BPC, b"BPC");
    key!(BS, b"BS");
    key!(BTN, b"Btn");
    key!(BYTERANGE, b"ByteRange");

    // C
    key!(C, b"C");
    key!(C0, b"C0");
    key!(C1, b"C1");
    key!(CA, b"CA");
    key!(CA_NS, b"ca");
    key!(CALGRAY, b"CalGray");
    key!(CALRGB, b"CalRGB");
    key!(CALCMYK, b"CalCMYK");
    key!(CAP, b"Cap");
    key!(CAP_HEIGHT, b"CapHeight");
    key!(CATALOG, b"Catalog");
    key!(CCITTFAX_DECODE, b"CCITTFaxDecode");
    key!(CCITTFAX_DECODE_ABBREVIATION, b"CCF");
    key!(CENTER_WINDOW, b"CenterWindow");
    key!(CERT, b"Cert");
    key!(CERTS, b"Certs");
    key!(CF, b"CF");
    key!(CFM, b"CFM");
    key!(CH, b"Ch");
    key!(CHAR_PROCS, b"CharProcs");
    key!(CHAR_SET, b"CharSet");
    key!(CHECK_SUM, b"CheckSum");
    key!(CI, b"CI");
    key!(CICI_SIGNIT, b"CICI.SignIt");
    key!(CID_FONT_TYPE0, b"CIDFontType0");
    key!(CID_FONT_TYPE0C, b"CIDFontType0C");
    key!(CID_FONT_TYPE2, b"CIDFontType2");
    key!(CID_TO_GID_MAP, b"CIDToGIDMap");
    key!(CID_SET, b"CIDSet");
    key!(CIDSYSTEMINFO, b"CIDSystemInfo");
    key!(CL, b"CL");
    key!(CLASS_MAP, b"ClassMap");
    key!(CLR_F, b"ClrF");
    key!(CLR_FF, b"ClrFf");
    key!(CMAP, b"CMap");
    key!(CMAPNAME, b"CMapName");
    key!(CMYK, b"CMYK");
    key!(CO, b"CO");
    key!(COLOR, b"Color");
    key!(COLLECTION, b"Collection");
    key!(COLLECTION_ITEM, b"CollectionItem");
    key!(COLLECTION_FIELD, b"CollectionField");
    key!(COLLECTION_SCHEMA, b"CollectionSchema");
    key!(COLLECTION_SORT, b"CollectionSort");
    key!(COLLECTION_SUBITEM, b"CollectionSubitem");
    key!(COLOR_BURN, b"ColorBurn");
    key!(COLOR_DODGE, b"ColorDodge");
    key!(COLORANTS, b"Colorants");
    key!(COLORS, b"Colors");
    key!(COLORSPACE, b"ColorSpace");
    key!(COLUMNS, b"Columns");
    key!(COMPATIBLE, b"Compatible");
    key!(COMPONENTS, b"Components");
    key!(CONTACT_INFO, b"ContactInfo");
    key!(CONTENTS, b"Contents");
    key!(COORDS, b"Coords");
    key!(COUNT, b"Count");
    key!(CP, b"CP");
    key!(CREATION_DATE, b"CreationDate");
    key!(CREATOR, b"Creator");
    key!(CRL, b"CRL");
    key!(CRLS, b"CRLS");
    key!(CROP_BOX, b"CropBox");
    key!(CRYPT, b"Crypt");
    key!(CS, b"CS");
    key!(CYX, b"CYX");

    // D
    key!(D, b"D");
    key!(DA, b"DA");
    key!(DARKEN, b"Darken");
    key!(DATE, b"Date");
    key!(DCT_DECODE, b"DCTDecode");
    key!(DCT_DECODE_ABBREVIATION, b"DCT");
    key!(DECODE, b"Decode");
    key!(DECODE_PARMS, b"DecodeParms");
    key!(DEFAULT, b"default");
    key!(DEFAULT_CMYK, b"DefaultCMYK");
    key!(DEFAULT_CRYPT_FILTER, b"DefaultCryptFilter");
    key!(DEFAULT_GRAY, b"DefaultGray");
    key!(DEFAULT_RGB, b"DefaultRGB");
    key!(DESC, b"Desc");
    key!(DESCENDANT_FONTS, b"DescendantFonts");
    key!(DESCENT, b"Descent");
    key!(DEST, b"Dest");
    key!(DEST_OUTPUT_PROFILE, b"DestOutputProfile");
    key!(DESTS, b"Dests");
    key!(DEVICE_CMYK, b"DeviceCMYK");
    key!(DEVICE_GRAY, b"DeviceGray");
    key!(DEVICE_N, b"DeviceN");
    key!(DEVICE_RGB, b"DeviceRGB");
    key!(DI, b"Di");
    key!(DIFFERENCE, b"Difference");
    key!(DIFFERENCES, b"Differences");
    key!(DIGEST_METHOD, b"DigestMethod");
    key!(DIGEST_RIPEMD160, b"RIPEMD160");
    key!(DIGEST_SHA1, b"SHA1");
    key!(DIGEST_SHA256, b"SHA256");
    key!(DIGEST_SHA384, b"SHA384");
    key!(DIGEST_SHA512, b"SHA512");
    key!(DIRECTION, b"Direction");
    key!(DISPLAY_DOC_TITLE, b"DisplayDocTitle");
    key!(DL, b"DL");
    key!(DM, b"Dm");
    key!(DOC, b"Doc");
    key!(DOC_CHECKSUM, b"DocChecksum");
    key!(DOC_TIME_STAMP, b"DocTimeStamp");
    key!(DOCMDP, b"DocMDP");
    key!(DOCUMENT, b"Document");
    key!(DOMAIN, b"Domain");
    key!(DOS, b"DOS");
    key!(DP, b"DP");
    key!(DR, b"DR");
    key!(DS, b"DS");
    key!(DSS, b"DSS");
    key!(DUPLEX, b"Duplex");
    key!(DUR, b"Dur");
    key!(DV, b"DV");
    key!(DW, b"DW");
    key!(DW2, b"DW2");

    // E
    key!(E, b"E");
    key!(EARLY_CHANGE, b"EarlyChange");
    key!(EF, b"EF");
    key!(EMBEDDED_FDFS, b"EmbeddedFDFs");
    key!(EMBEDDED_FILE, b"EmbeddedFile");
    key!(EMBEDDED_FILES, b"EmbeddedFiles");
    key!(EMPTY, b"");
    key!(ENCODE, b"Encode");
    key!(ENCODED_BYTE_ALIGN, b"EncodedByteAlign");
    key!(ENCODING, b"Encoding");
    key!(ENCODING_90MS_RKSJ_H, b"90ms-RKSJ-H");
    key!(ENCODING_90MS_RKSJ_V, b"90ms-RKSJ-V");
    key!(ENCODING_ETEN_B5_H, b"ETen-B5-H");
    key!(ENCODING_ETEN_B5_V, b"ETen-B5-V");
    key!(ENCRYPT, b"Encrypt");
    key!(ENCRYPT_META_DATA, b"EncryptMetadata");
    key!(ENCRYPTED_PAYLOAD, b"EncryptedPayload");
    key!(END_OF_BLOCK, b"EndOfBlock");
    key!(END_OF_LINE, b"EndOfLine");
    key!(ENTRUST_PPKEF, b"Entrust.PPKEF");
    key!(EXCLUSION, b"Exclusion");
    key!(EXTENSIONS, b"Extensions");
    key!(EXTENSION_LEVEL, b"ExtensionLevel");
    key!(EX_DATA, b"ExData");
    key!(EXPORT, b"Export");
    key!(EXPORT_STATE, b"ExportState");
    key!(EXT_G_STATE, b"ExtGState");
    key!(EXTEND, b"Extend");
    key!(EXTENDS, b"Extends");

    // F
    key!(F, b"F");
    key!(F_DECODE_PARMS, b"FDecodeParms");
    key!(F_FILTER, b"FFilter");
    key!(FB, b"FB");
    key!(FDF, b"FDF");
    key!(FF, b"Ff");
    key!(FIELDS, b"Fields");
    key!(FILESPEC, b"Filespec");
    key!(FILTER, b"Filter");
    key!(FIRST, b"First");
    key!(FIRST_CHAR, b"FirstChar");
    key!(FIT_WINDOW, b"FitWindow");
    key!(FL, b"FL");
    key!(FLAGS, b"Flags");
    key!(FLATE_DECODE, b"FlateDecode");
    key!(FLATE_DECODE_ABBREVIATION, b"Fl");
    key!(FO, b"Fo");
    key!(FOLDERS, b"Folders");
    key!(FONT, b"Font");
    key!(FONT_BBOX, b"FontBBox");
    key!(FONT_DESC, b"FontDescriptor");
    key!(FONT_FAMILY, b"FontFamily");
    key!(FONT_FILE, b"FontFile");
    key!(FONT_FILE2, b"FontFile2");
    key!(FONT_FILE3, b"FontFile3");
    key!(FONT_MATRIX, b"FontMatrix");
    key!(FONT_NAME, b"FontName");
    key!(FONT_STRETCH, b"FontStretch");
    key!(FONT_WEIGHT, b"FontWeight");
    key!(FORM, b"Form");
    key!(FORMTYPE, b"FormType");
    key!(FRM, b"FRM");
    key!(FS, b"FS");
    key!(FT, b"FT");
    key!(FUNCTION, b"Function");
    key!(FUNCTION_TYPE, b"FunctionType");
    key!(FUNCTIONS, b"Functions");

    // G
    key!(G, b"G");
    key!(GAMMA, b"Gamma");
    key!(GROUP, b"Group");
    key!(GTS_PDFA1, b"GTS_PDFA1");

    // H
    key!(H, b"H");
    key!(HARD_LIGHT, b"HardLight");
    key!(HEIGHT, b"Height");
    key!(HELV, b"Helv");
    key!(HIDE_MENUBAR, b"HideMenubar");
    key!(HIDE_TOOLBAR, b"HideToolbar");
    key!(HIDE_WINDOWUI, b"HideWindowUI");
    key!(HUE, b"Hue");

    // I
    key!(I, b"I");
    key!(IC, b"IC");
    key!(ICC_BASED, b"ICCBased");
    key!(ID, b"ID");
    key!(ID_TREE, b"IDTree");
    key!(IDENTITY, b"Identity");
    key!(IDENTITY_H, b"Identity-H");
    key!(IDENTITY_V, b"Identity-V");
    key!(IF, b"IF");
    key!(ILLUSTRATOR, b"Illustrator");
    key!(IM, b"IM");
    key!(IMAGE, b"Image");
    key!(IMAGE_MASK, b"ImageMask");
    key!(INDEX, b"Index");
    key!(INDEXED, b"Indexed");
    key!(INFO, b"Info");
    key!(INKLIST, b"InkList");
    key!(INTENT, b"Intent");
    key!(INTERPOLATE, b"Interpolate");
    key!(IRT, b"IRT");
    key!(IT, b"IT");
    key!(ITALIC_ANGLE, b"ItalicAngle");
    key!(ISSUER, b"Issuer");
    key!(IX, b"IX");

    // J
    key!(JAVA_SCRIPT, b"JavaScript");
    key!(JBIG2_DECODE, b"JBIG2Decode");
    key!(JBIG2_GLOBALS, b"JBIG2Globals");
    key!(JPX_DECODE, b"JPXDecode");
    key!(JS, b"JS");

    // K
    key!(K, b"K");
    key!(KEYWORDS, b"Keywords");
    key!(KEY_USAGE, b"KeyUsage");
    key!(KIDS, b"Kids");

    // L
    key!(L, b"L");
    key!(LAB, b"Lab");
    key!(LANG, b"Lang");
    key!(LAST, b"Last");
    key!(LAST_CHAR, b"LastChar");
    key!(LAST_MODIFIED, b"LastModified");
    key!(LC, b"LC");
    key!(LE, b"LE");
    key!(LEADING, b"Leading");
    key!(LEGAL_ATTESTATION, b"LegalAttestation");
    key!(LENGTH, b"Length");
    key!(LENGTH1, b"Length1");
    key!(LENGTH2, b"Length2");
    key!(LENGTH3, b"Length3");
    key!(LIGHTEN, b"Lighten");
    key!(LIMITS, b"Limits");
    key!(LINEARIZED, b"Linearized");
    key!(LJ, b"LJ");
    key!(LL, b"LL");
    key!(LLE, b"LLE");
    key!(LLO, b"LLO");
    key!(LOCATION, b"Location");
    key!(LUMINOSITY, b"Luminosity");
    key!(LW, b"LW");
    key!(LZW_DECODE, b"LZWDecode");
    key!(LZW_DECODE_ABBREVIATION, b"LZW");

    // M
    key!(M, b"M");
    key!(MAC, b"Mac");
    key!(MAC_EXPERT_ENCODING, b"MacExpertEncoding");
    key!(MAC_ROMAN_ENCODING, b"MacRomanEncoding");
    key!(MARK_INFO, b"MarkInfo");
    key!(MASK, b"Mask");
    key!(MATRIX, b"Matrix");
    key!(MATTE, b"Matte");
    key!(MAX_LEN, b"MaxLen");
    key!(MAX_WIDTH, b"MaxWidth");
    key!(MCID, b"MCID");
    key!(MDP, b"MDP");
    key!(MEDIA_BOX, b"MediaBox");
    key!(MEASURE, b"Measure");
    key!(METADATA, b"Metadata");
    key!(MISSING_WIDTH, b"MissingWidth");
    key!(MIX, b"Mix");
    key!(MK, b"MK");
    key!(ML, b"ML");
    key!(MM_TYPE1, b"MMType1");
    key!(MOD_DATE, b"ModDate");
    key!(MULTIPLY, b"Multiply");

    // N
    key!(N, b"N");
    key!(NAME, b"Name");
    key!(NAMES, b"Names");
    key!(NAVIGATOR, b"Navigator");
    key!(NEED_APPEARANCES, b"NeedAppearances");
    key!(NEW_WINDOW, b"NewWindow");
    key!(NEXT, b"Next");
    key!(NM, b"NM");
    key!(NON_EFONT_NO_WARN, b"NonEFontNoWarn");
    key!(NON_FULL_SCREEN_PAGE_MODE, b"NonFullScreenPageMode");
    key!(NONE, b"None");
    key!(NORMAL, b"Normal");
    key!(NUMS, b"Nums");

    // O
    key!(O, b"O");
    key!(OBJ, b"Obj");
    key!(OBJR, b"OBJR");
    key!(OBJ_STM, b"ObjStm");
    key!(OC, b"OC");
    key!(OCG, b"OCG");
    key!(OCGS, b"OCGs");
    key!(OCMD, b"OCMD");
    key!(OCPROPERTIES, b"OCProperties");
    key!(OCSP, b"OCSP");
    key!(OCSPS, b"OCSPs");
    key!(OE, b"OE");
    key!(OID, b"OID");
    key!(OFF, b"OFF");
    key!(ON, b"ON");
    key!(OP, b"OP");
    key!(OP_NS, b"op");
    key!(OPEN_ACTION, b"OpenAction");
    key!(OPEN_TYPE, b"OpenType");
    key!(OPI, b"OPI");
    key!(OPM, b"OPM");
    key!(OPT, b"Opt");
    key!(ORDER, b"Order");
    key!(ORDERING, b"Ordering");
    key!(OS, b"OS");
    key!(OUTLINES, b"Outlines");
    key!(OUTPUT_CONDITION, b"OutputCondition");
    key!(OUTPUT_CONDITION_IDENTIFIER, b"OutputConditionIdentifier");
    key!(OUTPUT_INTENT, b"OutputIntent");
    key!(OUTPUT_INTENTS, b"OutputIntents");
    key!(OVERLAY, b"Overlay");

    // P
    key!(P, b"P");
    key!(PA, b"PA");
    key!(PAGE, b"Page");
    key!(PAGE_LABELS, b"PageLabels");
    key!(PAGE_LAYOUT, b"PageLayout");
    key!(PAGE_MODE, b"PageMode");
    key!(PAGES, b"Pages");
    key!(PAINT_TYPE, b"PaintType");
    key!(PANOSE, b"Panose");
    key!(PARAMS, b"Params");
    key!(PARENT, b"Parent");
    key!(PARENT_TREE, b"ParentTree");
    key!(PARENT_TREE_NEXT_KEY, b"ParentTreeNextKey");
    key!(PART, b"Part");
    key!(PATH, b"Path");
    key!(PATTERN, b"Pattern");
    key!(PATTERN_TYPE, b"PatternType");
    key!(PC, b"PC");
    key!(PDF_DOC_ENCODING, b"PDFDocEncoding");
    key!(PERMS, b"Perms");
    key!(PERCEPTUAL, b"Perceptual");
    key!(PIECE_INFO, b"PieceInfo");
    key!(PG, b"Pg");
    key!(PI, b"PI");
    key!(PO, b"PO");
    key!(POPUP, b"Popup");
    key!(PRE_RELEASE, b"PreRelease");
    key!(PREDICTOR, b"Predictor");
    key!(PREV, b"Prev");
    key!(PRINT, b"Print");
    key!(PRINT_AREA, b"PrintArea");
    key!(PRINT_CLIP, b"PrintClip");
    key!(PRINT_SCALING, b"PrintScaling");
    key!(PRINT_STATE, b"PrintState");
    key!(PRIVATE, b"Private");
    key!(PROC_SET, b"ProcSet");
    key!(PROCESS, b"Process");
    key!(PRODUCER, b"Producer");
    key!(PROP_BUILD, b"Prop_Build");
    key!(PROPERTIES, b"Properties");
    key!(PS, b"PS");
    key!(PT_DATA, b"PtData");
    key!(PUB_SEC, b"PubSec");
    key!(PV, b"PV");

    // Q
    key!(Q, b"Q");
    key!(QUADPOINTS, b"QuadPoints");

    // R
    key!(R, b"R");
    key!(RANGE, b"Range");
    key!(RC, b"RC");
    key!(RD, b"RD");
    key!(REASON, b"Reason");
    key!(REASONS, b"Reasons");
    key!(RECIPIENTS, b"Recipients");
    key!(RECT, b"Rect");
    key!(REF, b"Ref");
    key!(REFERENCE, b"Reference");
    key!(REGISTRY, b"Registry");
    key!(REGISTRY_NAME, b"RegistryName");
    key!(RELATIVE_COLORIMETRIC, b"RelativeColorimetric");
    key!(RENAME, b"Rename");
    key!(REPEAT, b"Repeat");
    key!(RES_FORK, b"ResFork");
    key!(RESOURCES, b"Resources");
    key!(RGB, b"RGB");
    key!(RI, b"RI");
    key!(ROLE_MAP, b"RoleMap");
    key!(ROOT, b"Root");
    key!(ROTATE, b"Rotate");
    key!(ROWS, b"Rows");
    key!(RT, b"RT");
    key!(RUN_LENGTH_DECODE, b"RunLengthDecode");
    key!(RUN_LENGTH_DECODE_ABBREVIATION, b"RL");
    key!(RV, b"RV");

    // S
    key!(S, b"S");
    key!(SA, b"SA");
    key!(SATURATION, b"Saturation");
    key!(SCHEMA, b"Schema");
    key!(SCREEN, b"Screen");
    key!(SE, b"SE");
    key!(SEPARATION, b"Separation");
    key!(SET_F, b"SetF");
    key!(SET_FF, b"SetFf");
    key!(SHADING, b"Shading");
    key!(SHADING_TYPE, b"ShadingType");
    key!(SIG, b"Sig");
    key!(SIG_FLAGS, b"SigFlags");
    key!(SIG_REF, b"SigRef");
    key!(SIZE, b"Size");
    key!(SM, b"SM");
    key!(SMASK, b"SMask");
    key!(SMASK_IN_DATA, b"SMaskInData");
    key!(SOFT_LIGHT, b"SoftLight");
    key!(SORT, b"Sort");
    key!(SOUND, b"Sound");
    key!(SPLIT, b"Split");
    key!(SS, b"SS");
    key!(ST, b"St");
    key!(STANDARD_ENCODING, b"StandardEncoding");
    key!(STATE, b"State");
    key!(STATE_MODEL, b"StateModel");
    key!(STATUS, b"Status");
    key!(STD_CF, b"StdCF");
    key!(STEM_H, b"StemH");
    key!(STEM_V, b"StemV");
    key!(STM_F, b"StmF");
    key!(STR_F, b"StrF");
    key!(STRUCT_ELEM, b"StructElem");
    key!(STRUCT_PARENT, b"StructParent");
    key!(STRUCT_PARENTS, b"StructParents");
    key!(STRUCT_TREE_ROOT, b"StructTreeRoot");
    key!(STYLE, b"Style");
    key!(SUB_FILTER, b"SubFilter");
    key!(SUBJ, b"Subj");
    key!(SUBJECT, b"Subject");
    key!(SUBJECT_DN, b"SubjectDN");
    key!(SUBTYPE, b"Subtype");
    key!(SUPPLEMENT, b"Supplement");
    key!(SV, b"SV");
    key!(SV_CERT, b"SVCert");
    key!(SW, b"SW");
    key!(SY, b"Sy");
    key!(SYNCHRONOUS, b"Synchronous");
    key!(T, b"T");
    key!(TARGET, b"Target");
    key!(TEMPLATES, b"Templates");
    key!(THREAD, b"Thread");
    key!(THREADS, b"Threads");
    key!(THREE_DD, b"3DD");
    key!(THUMB, b"Thumb");
    key!(TI, b"TI");
    key!(TILING_TYPE, b"TilingType");
    key!(TIME_STAMP, b"TimeStamp");
    key!(TITLE, b"Title");
    key!(TK, b"TK");
    key!(TM, b"TM");
    key!(TO_UNICODE, b"ToUnicode");
    key!(TR, b"TR");
    key!(TR2, b"TR2");
    key!(TRAPPED, b"Trapped");
    key!(TRANS, b"Trans");
    key!(TRANSFORM_METHOD, b"TransformMethod");
    key!(TRANSFORM_PARAMS, b"TransformParams");
    key!(TRANSPARENCY, b"Transparency");
    key!(TREF, b"TRef");
    key!(TRIM_BOX, b"TrimBox");
    key!(TRUE_TYPE, b"TrueType");
    key!(TRUSTED_MODE, b"TrustedMode");
    key!(TU, b"TU");
    key!(TX, b"Tx");
    key!(TYPE, b"Type");
    key!(TYPE0, b"Type0");
    key!(TYPE1, b"Type1");
    key!(TYPE3, b"Type3");

    // U
    key!(U, b"U");
    key!(UE, b"UE");
    key!(UF, b"UF");
    key!(UNCHANGED, b"Unchanged");
    key!(UNIX, b"Unix");
    key!(URI, b"URI");
    key!(URL, b"URL");
    key!(URL_TYPE, b"URLType");
    key!(USAGE, b"Usage");
    key!(USE_CMAP, b"UseCMap");
    key!(USER_UNIT, b"UserUnit");

    // V
    key!(V, b"V");
    key!(VE, b"VE");
    key!(VERISIGN_PPKVS, b"VeriSign.PPKVS");
    key!(VERSION, b"Version");
    key!(VERTICES, b"Vertices");
    key!(VERTICES_PER_ROW, b"VerticesPerRow");
    key!(VIEW, b"View");
    key!(VIEW_AREA, b"ViewArea");
    key!(VIEW_CLIP, b"ViewClip");
    key!(VIEW_STATE, b"ViewState");
    key!(VIEWER_PREFERENCES, b"ViewerPreferences");
    key!(VOLUME, b"Volume");
    key!(VP, b"VP");
    key!(VRI, b"VRI");

    // W
    key!(W, b"W");
    key!(W2, b"W2");
    key!(WC, b"WC");
    key!(WHITE_POINT, b"WhitePoint");
    key!(WIDGET, b"Widget");
    key!(WIDTH, b"Width");
    key!(WIDTHS, b"Widths");
    key!(WIN, b"Win");
    key!(WIN_ANSI_ENCODING, b"WinAnsiEncoding");
    key!(WMODE, b"WMode");
    key!(WP, b"WP");
    key!(WS, b"WS");

    // X
    key!(X, b"X");
    key!(XFA, b"XFA");
    key!(X_STEP, b"XStep");
    key!(XHEIGHT, b"XHeight");
    key!(XOBJECT, b"XObject");
    key!(XREF, b"XRef");
    key!(XREF_STM, b"XRefStm");

    // Y
    key!(Y, b"Y");
    key!(Y_STEP, b"YStep");
    key!(YES, b"Yes");

    // Z
    key!(ZA_DB, b"ZaDb");
}

#[cfg(test)]
mod tests {
    use crate::object::Number;
    use crate::object::dict::keys::{COLORSPACE, EXT_G_STATE, FONT, PROC_SET};
    use crate::object::dict::{Dict, InlineImageDict};
    use crate::object::string;
    use crate::object::{Name, ObjRef};
    use crate::reader::{Reader, ReaderContext};

    fn dict_impl(data: &[u8]) -> Option<Dict<'_>> {
        Reader::new(data).read_with_context::<Dict>(&ReaderContext::dummy())
    }

    #[test]
    fn empty_dict_1() {
        let dict_data = b"<<>>";
        let dict = dict_impl(dict_data).unwrap();

        assert_eq!(dict.len(), 0);
    }

    #[test]
    fn empty_dict_2() {
        let dict_data = b"<<   \n >>";
        let dict = dict_impl(dict_data).unwrap();

        assert_eq!(dict.len(), 0);
    }

    #[test]
    fn dict_1() {
        let dict_data = b"<<  /Hi 34.0 >>";
        let dict = dict_impl(dict_data).unwrap();

        assert_eq!(dict.len(), 1);
        assert!(dict.get::<Number>(Name::new(b"Hi")).is_some());
    }

    #[test]
    fn dict_2() {
        let dict_data = b"<<  /Hi \n 34.0 /Second true >>";
        let dict = dict_impl(dict_data).unwrap();

        assert_eq!(dict.len(), 2);
        assert!(dict.get::<Number>(Name::new(b"Hi")).is_some());
        assert!(dict.get::<bool>(Name::new(b"Second")).is_some());
    }

    #[test]
    fn dict_complex() {
        let data = "<< /Type /Example
/Subtype /DictionaryExample
/Version 0.01
/IntegerItem 12
/StringItem ( a string )
/Subdictionary << /Item1 0.4
                /Item2 true
                /LastItem ( not ! )
                /VeryLastItem ( OK )
                >>
>>";

        let dict = Reader::new(data.as_bytes())
            .read_with_context::<Dict>(&ReaderContext::dummy())
            .unwrap();
        assert_eq!(dict.len(), 6);
        assert!(dict.get::<Name>(Name::new(b"Type")).is_some());
        assert!(dict.get::<Name>(Name::new(b"Subtype")).is_some());
        assert!(dict.get::<Number>(Name::new(b"Version")).is_some());
        assert!(dict.get::<i32>(Name::new(b"IntegerItem")).is_some());
        assert!(
            dict.get::<string::String>(Name::new(b"StringItem"))
                .is_some()
        );
        assert!(dict.get::<Dict>(Name::new(b"Subdictionary")).is_some());
    }

    #[test]
    fn dict_with_trailing() {
        let dict_data = b"<<  /Hi 67.0  >>trailing data";
        let dict = dict_impl(dict_data).unwrap();

        assert_eq!(dict.len(), 1);
    }

    #[test]
    fn dict_with_comment() {
        let dict_data = b"<<  /Hi % A comment \n 67.0 % Another comment \n >>";
        let dict = dict_impl(dict_data).unwrap();

        assert_eq!(dict.len(), 1);
    }

    #[test]
    fn inline_dict() {
        let dict_data = b"/W 17 /H 17 /CS /RGB /BPC 8 /F [ /A85 /LZW ] ID ";

        let dict = Reader::new(&dict_data[..])
            .read_with_context::<InlineImageDict>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(dict.get_dict().len(), 5);
    }

    #[test]
    fn dict_with_escaped_name() {
        let dict_data = b"<< /PANTONE#20104#20C 234 >>";
        let dict = dict_impl(dict_data).unwrap();

        assert!(dict.contains_key(b"PANTONE 104 C".as_ref()));
    }

    #[test]
    fn garbage_in_between() {
        let dict_data = b"<< 
/ProcSet [ /PDF /Text ] 
/Font << /F4 31 0 R /F6 23 0 R >> 
/ExtGState << /GS2 14 0 R
2000
 /GS3 15 0 R >> 
/ColorSpace << /Cs5 13 0 R >> 
>> ";
        let dict = dict_impl(dict_data).unwrap();

        assert!(dict.contains_key(PROC_SET));
        assert!(dict.contains_key(FONT));
        assert!(dict.contains_key(EXT_G_STATE));
        assert!(dict.contains_key(COLORSPACE));

        let Some(dict) = dict.get::<Dict>(EXT_G_STATE) else {
            panic!("failed to parse ext g state");
        };

        assert_eq!(dict.get_ref("GS2".as_ref()), Some(ObjRef::new(14, 0)));
        assert_eq!(dict.get_ref("GS3".as_ref()), Some(ObjRef::new(15, 0)));
    }
}
