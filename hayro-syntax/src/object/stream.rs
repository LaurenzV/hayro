//! Streams.

use crate::crypto::DecryptionTarget;
use crate::filter::Filter;
use crate::object;
use crate::object::Dict;
use crate::object::Name;
use crate::object::dict::keys::{DECODE_PARMS, DP, F, FILTER, IDENTITY, LENGTH, NAME, TYPE};
use crate::object::{Array, ObjectIdentifier};
use crate::object::{Object, ObjectLike, ObjectRefLike};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use crate::trivia::is_white_space_character;
use crate::util::{OptionLog, find_needle};
use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt::{Debug, Display, Formatter};
use core::ops::Deref;
use smallvec::SmallVec;

struct FiltersAndParams<'a> {
    filters: SmallVec<[Filter; 2]>,
    params: SmallVec<[Dict<'a>; 2]>,
}

/// A stream of arbitrary data.
#[derive(Clone)]
pub struct Stream<'a> {
    dict: Dict<'a>,
    data: &'a [u8],
}

impl PartialEq for Stream<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.dict == other.dict && self.data == other.data
    }
}

/// Additional parameters for decoding images.
#[derive(Clone, PartialEq, Default)]
pub struct ImageDecodeParams {
    /// Whether the color space of the image is an indexed color space.
    pub is_indexed: bool,
    /// The bits per component of the image, if that information is available.
    pub bpc: Option<u8>,
    /// The components per channel of the image, if that information is available.
    pub num_components: Option<u8>,
    /// A target resolution for the image. Note that this is only a hint so that
    /// in case it's possible, a version of the image will be extracted that
    /// is as close as possible to the hinted dimension.
    pub target_dimension: Option<(u32, u32)>,
    /// The width of the image as indicated by the image dictionary.
    pub width: u32,
    /// The height of the image as indicated by the image dictionary.
    pub height: u32,
}

impl<'a> Stream<'a> {
    pub(crate) fn new(data: &'a [u8], dict: Dict<'a>) -> Self {
        Self { dict, data }
    }

    fn filters_and_params(&self) -> FiltersAndParams<'a> {
        let mut collected_filters = SmallVec::new();
        let mut collected_params = SmallVec::new();

        if let Some(filter) = self
            .dict
            .get::<Name<'_>>(F)
            .or_else(|| self.dict.get::<Name<'_>>(FILTER))
            .and_then(Filter::from_name)
        {
            let params = self
                .dict
                .get::<Dict<'_>>(DP)
                .or_else(|| self.dict.get::<Dict<'_>>(DECODE_PARMS))
                .unwrap_or_default();

            collected_filters.push(filter);
            collected_params.push(params);
        } else if let Some(filters) = self
            .dict
            .get::<Array<'_>>(F)
            .or_else(|| self.dict.get::<Array<'_>>(FILTER))
        {
            let filters = filters.iter::<Name<'_>>().map(Filter::from_name);
            let mut params = self
                .dict
                .get::<Array<'_>>(DP)
                .or_else(|| self.dict.get::<Array<'_>>(DECODE_PARMS))
                .map(|a| a.iter::<Object<'_>>());

            for filter in filters {
                let params = params
                    .as_mut()
                    .and_then(|p| p.next())
                    .and_then(|p| p.into_dict())
                    .unwrap_or_default();

                if let Some(filter) = filter {
                    collected_filters.push(filter);
                    collected_params.push(params);
                }
            }
        }

        FiltersAndParams {
            filters: collected_filters,
            params: collected_params,
        }
    }

    /// Return the raw, decrypted data of the stream.
    ///
    /// Stream filters will not be applied. /Crypt filters are: the data are
    /// decrypted with the crypt filter the first one names, and the others pass the
    /// data through, so the caller drops them all.
    pub fn raw_data(&self) -> Cow<'a, [u8]> {
        let decrypted = match self.decryption() {
            Ok(Some((id, name))) => {
                self.dict
                    .ctx()
                    .xref()
                    .decrypt(id, self.data, Self::target(&name))
            }
            Ok(None) => return Cow::Borrowed(self.data),
            Err(_) => None,
        };

        // TODO: MAybe an error would be better?
        Cow::Owned(decrypted.unwrap_or_default())
    }

    // How the data is decrypted: `None` where they are stored as is, else with the
    // key of the object, by the document's stream filter (no name) or the crypt
    // filter named. In an encrypted document a stream that is no cross-reference
    // stream is decrypted by the document's stream filter, unless it has /Crypt
    // filters, which replace it (`pdf_open_raw_filter` in MuPDF's `pdf-stream.c`).
    // A /Crypt filter without /Name or named /Identity passes the data through;
    // another names one of the document's crypt filters, which decrypts the data
    // (`build_filter` there). It must be the first filter, as the PDF specification
    // requires: decrypting data another filter decoded is not supported.
    fn decryption(&self) -> Result<Option<(ObjectIdentifier, Option<Name<'a>>)>, DecodeFailure> {
        let ctx = self.dict.ctx();
        if !ctx.xref().needs_decryption(ctx)
            || self
                .dict
                .get::<object::String<'_>>(TYPE)
                .is_some_and(|t| t.as_ref() == b"XRef")
        {
            return Ok(None);
        }
        let FiltersAndParams { filters, params } = self.filters_and_params();
        if !filters.contains(&Filter::Crypt) {
            // `None` should not be reachable, as streams always have IDs.
            return Ok(self.dict.obj_id().map(|id| (id, None)));
        }
        let mut named = None;
        for (i, (filter, params)) in filters.iter().zip(&params).enumerate() {
            if *filter != Filter::Crypt {
                continue;
            }
            match params.get::<Name<'a>>(NAME) {
                Some(name) if name.deref() != IDENTITY => {
                    let id = self.dict.obj_id().filter(|_| i == 0);
                    named = Some((id.ok_or(DecodeFailure::Decryption)?, Some(name)));
                }
                _ => {}
            }
        }
        Ok(named)
    }

    fn target<'n>(name: &'n Option<Name<'_>>) -> DecryptionTarget<'n> {
        name.as_deref()
            .map_or(DecryptionTarget::Stream, DecryptionTarget::Named)
    }

    /// Return the raw, underlying dictionary of the stream.
    pub fn dict(&self) -> &Dict<'a> {
        &self.dict
    }

    /// Return the object identifier of the stream.
    pub fn obj_id(&self) -> ObjectIdentifier {
        self.dict.obj_id().unwrap_or_else(|| {
            // In theory shouldn't ever happen, but could theoretically be
            // triggered in a crafted PDF.
            ObjectIdentifier::new(0, 0)
        })
    }

    /// Return the filters that are applied to the stream.
    pub fn filters(&self) -> SmallVec<[Filter; 2]> {
        self.filters_and_params().filters
    }

    /// Return the decoded data of the stream.
    ///
    /// Note that the result of this method will not be cached, so calling it multiple
    /// times is expensive.
    pub fn decoded(&self) -> Result<Cow<'a, [u8]>, DecodeFailure> {
        self.decoded_image(&ImageDecodeParams::default())
            .map(|r| r.data)
    }

    /// Return the decoded data of the stream, and return image metadata
    /// if available.
    pub fn decoded_image(
        &self,
        image_params: &ImageDecodeParams,
    ) -> Result<FilterResult<'a>, DecodeFailure> {
        let data = self.raw_data();
        let filters_and_params = self.filters_and_params();

        let mut current: Option<FilterResult<'a>> = None;

        for (filter, params) in filters_and_params
            .filters
            .iter()
            .zip(filters_and_params.params.iter())
        {
            if *filter == Filter::Crypt {
                // Decrypted with the raw data.
                continue;
            }
            let new = filter.apply(
                current.as_ref().map(|c| c.data.as_ref()).unwrap_or(&data),
                params,
                image_params,
            )?;
            current = Some(new);
        }

        Ok(current.unwrap_or(FilterResult {
            data,
            image_data: None,
        }))
    }
}

impl Debug for Stream<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Stream (len: {:?})", self.data.len())
    }
}

impl Display for Stream<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Stream (len: {})", self.data.len())
    }
}

impl Skippable for Stream<'_> {
    fn skip(_: &mut Reader<'_>, _: bool) -> Option<()> {
        // A stream can never appear in a dict/array, so it should never be skipped.
        warn!("attempted to skip a stream object");

        None
    }
}

impl<'a> Readable<'a> for Stream<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let dict = r.read_with_context::<Dict<'_>>(ctx)?;

        if dict.contains_key(F) {
            warn!("encountered stream referencing external file, which is unsupported");

            return None;
        }

        let offset = r.offset();
        parse_proper(r, &dict)
            .or_else(|| {
                warn!("failed to parse stream, trying to parse it manually");

                r.jump(offset);
                parse_fallback(r, &dict)
            })
            .error_none("was unable to manually parse the stream")
    }
}

#[derive(Debug, Copy, Clone)]
/// A failure that can occur during decoding a data stream.
pub enum DecodeFailure {
    /// An image stream failed to decode.
    ImageDecode,
    /// A data stream failed to decode.
    StreamDecode,
    /// A failure occurred while decrypting a file.
    Decryption,
    /// An unknown failure occurred.
    Unknown,
}

/// An image color space.
#[derive(Debug, Copy, Clone)]
pub enum ImageColorSpace {
    /// Grayscale color space.
    Gray,
    /// RGB color space.
    Rgb,
    /// CMYK color space.
    Cmyk,
    /// An unknown color space.
    Unknown(u8),
}

/// Additional data that is extracted from some image streams.
pub struct ImageData {
    /// An optional alpha channel of the image.
    pub alpha: Option<Vec<u8>>,
    /// The color space of the image.
    pub color_space: Option<ImageColorSpace>,
    /// The bits per component of the image.
    pub bits_per_component: u8,
    /// The width of the image.
    pub width: u32,
    /// The height of the image.
    pub height: u32,
}

/// The result of applying a filter.
pub struct FilterResult<'a> {
    /// The decoded data.
    pub data: Cow<'a, [u8]>,
    /// Additional data that is extracted from JPX image streams.
    pub image_data: Option<ImageData>,
}

impl FilterResult<'_> {
    pub(crate) fn from_data(data: Vec<u8>) -> Self {
        Self {
            data: Cow::Owned(data),
            image_data: None,
        }
    }
}

fn parse_proper<'a>(r: &mut Reader<'a>, dict: &Dict<'a>) -> Option<Stream<'a>> {
    let length = dict.get::<u32>(LENGTH)?;

    r.skip_white_spaces_and_comments();
    r.forward_tag(b"stream")?;
    r.forward_tag(b"\n")
        .or_else(|| r.forward_tag(b"\r\n"))
        .or_else(|| r.forward_tag(b"\r"))?;
    let data = r.read_bytes(length as usize)?;
    r.skip_white_spaces();
    r.forward_tag(b"endstream")?;

    Some(Stream::new(data, dict.clone()))
}

fn parse_fallback<'a>(r: &mut Reader<'a>, dict: &Dict<'a>) -> Option<Stream<'a>> {
    r.skip_white_spaces_and_comments();
    r.forward_tag(b"stream")?;

    r.forward_tag(b"\n")
        .or_else(|| r.forward_tag(b"\r\n"))
        // Technically not allowed, but no reason to not try it.
        .or_else(|| r.forward_tag(b"\r"))?;

    // TODO: This search can take quadratic time in total when many streams with a wrong length
    // have no `endstream` after them, or only a distant one.
    let tail = r.tail()?;
    let endstream_offset = find_needle(tail, b"endstream")?;
    let data_end = trim_trailing_ascii_whitespace(&tail[..endstream_offset]);
    let data = tail.get(..data_end)?;

    r.read_bytes(endstream_offset)?;
    r.skip_white_spaces();
    r.forward_tag(b"endstream")?;

    Some(Stream::new(data, dict.clone()))
}

fn trim_trailing_ascii_whitespace(data: &[u8]) -> usize {
    let mut end = data.len();

    while data
        .get(end.wrapping_sub(1))
        .copied()
        .is_some_and(is_white_space_character)
    {
        end -= 1;
    }

    end
}

impl<'a> TryFrom<Object<'a>> for Stream<'a> {
    type Error = ();

    fn try_from(value: Object<'a>) -> Result<Self, Self::Error> {
        match value {
            Object::Stream(s) => Ok(s),
            _ => Err(()),
        }
    }
}

impl<'a> ObjectLike<'a> for Stream<'a> {}
impl<'a> ObjectRefLike<'a> for Stream<'a> {
    fn cast_ref<'b>(obj: &'b Object<'a>) -> Option<&'b Self> {
        match obj {
            Object::Stream(stream) => Some(stream),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::object::Stream;
    use crate::reader::Reader;
    use crate::reader::{ReaderContext, ReaderExt};

    #[test]
    fn display() {
        let mut reader = Reader::new(b"<< /Length 3 >> stream\nabc\nendstream");
        let stream = reader
            .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(format!("{stream}"), "Stream (len: 3)");
    }

    #[test]
    fn stream() {
        let data = b"<< /Length 10 >> stream\nabcdefghij\nendstream";
        let mut r = Reader::new(data);
        let stream = r
            .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(stream.data, b"abcdefghij");
    }

    #[test]
    fn stream_fallback() {
        let data = b"<< /Length 999 >> stream\nabcdefghij\nendstream";
        let mut r = Reader::new(data);
        let stream = r
            .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(stream.data, b"abcdefghij");
    }

    mod crypt {
        use crate::crypto::fixture::{Part, aes128};
        use crate::object::{ObjectIdentifier, Stream};
        use crate::pdf::Pdf;
        use alloc::format;
        use alloc::string::String;
        use alloc::vec::Vec;

        const RED: &[u8] = b"1 0 0 rg 0 0 10 10 re f";
        const BLUE: &[u8] = b"0 0 1 rg 10 0 10 10 re f";
        /// zlib of "0 1 0 rg 20 0 10 10 re f".
        const GREEN_FLATE: &[u8] = &[
            0x78, 0x9c, 0x33, 0x50, 0x30, 0x54, 0x30, 0x50, 0x28, 0x4a, 0x57, 0x30, 0x32, 0x00,
            0xd2, 0x86, 0x60, 0x54, 0x94, 0xaa, 0x90, 0x06, 0x00, 0x3b, 0x65, 0x05, 0x1c,
        ];

        const CATALOG: Part<'_> = Part::Plain("<< /Type /Catalog /Pages 2 0 R >>");
        const TREE: Part<'_> = Part::Plain("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");

        fn page(contents: &str) -> String {
            format!("<< /Type /Page /MediaBox [0 0 100 100] /Contents {contents} >>")
        }

        /// The page's content.
        fn read(file: Vec<u8>) -> Vec<u8> {
            let pdf = Pdf::new(file).unwrap();
            pdf.pages()[0].page_stream().unwrap_or_default().to_vec()
        }

        fn joined(parts: &[&[u8]]) -> Vec<u8> {
            parts
                .iter()
                .flat_map(|p| p.iter().chain(b" "))
                .copied()
                .collect()
        }

        #[test]
        fn an_identity_crypt_filter_leaves_its_stream_unencrypted() {
            for dict in [
                "/Filter /Crypt /DecodeParms << /Name /Identity >>",
                "/Filter /Crypt",
                "/Filter [/Crypt] /DecodeParms [<< /Name /Identity >>]",
            ] {
                let page = page("[4 0 R 5 0 R]");
                let file = aes128(&[
                    CATALOG,
                    TREE,
                    Part::Plain(&page),
                    Part::Stream {
                        dict,
                        data: RED,
                        encrypt: false,
                    },
                    Part::Stream {
                        dict: "",
                        data: BLUE,
                        encrypt: true,
                    },
                ]);
                assert_eq!(read(file), joined(&[RED, BLUE]), "{dict}");
            }
        }

        #[test]
        fn an_identity_crypt_filter_goes_before_the_others() {
            let page = page("4 0 R");
            let file = aes128(&[
                CATALOG,
                TREE,
                Part::Plain(&page),
                Part::Stream {
                    dict: "/Filter [/Crypt /FlateDecode] /DecodeParms [<< /Name /Identity >> null]",
                    data: GREEN_FLATE,
                    encrypt: false,
                },
            ]);
            assert_eq!(read(file), b"0 1 0 rg 20 0 10 10 re f");
        }

        // A crypt filter naming one of the document's decrypts with it, as the
        // document's stream filter does (MuPDF's `pdf_open_crypt_with_filter`).
        #[test]
        fn a_named_crypt_filter_decrypts_its_stream() {
            let page = page("[4 0 R 5 0 R]");
            let file = aes128(&[
                CATALOG,
                TREE,
                Part::Plain(&page),
                Part::Stream {
                    dict: "/Filter /Crypt /DecodeParms << /Name /StdCF >>",
                    data: RED,
                    encrypt: true,
                },
                Part::Stream {
                    dict: "/Filter [/Crypt /FlateDecode] /DecodeParms [<< /Name /StdCF >> null]",
                    data: GREEN_FLATE,
                    encrypt: true,
                },
            ]);
            assert_eq!(
                read(file.clone()),
                joined(&[RED, b"0 1 0 rg 20 0 10 10 re f"])
            );
            // The raw data a writer copies without the /Crypt filter are decrypted.
            let pdf = Pdf::new(file).unwrap();
            let raw = |n| {
                let stream = pdf
                    .xref()
                    .get::<Stream<'_>>(ObjectIdentifier::new(n, 0))
                    .unwrap();
                stream.raw_data().to_vec()
            };
            assert_eq!(raw(4), RED);
            assert_eq!(raw(5), GREEN_FLATE);
        }

        // A crypt filter the document does not define, and one after another filter,
        // which is not supported, fail to decrypt.
        #[test]
        fn a_crypt_filter_that_cannot_decrypt_leaves_no_data() {
            for dict in [
                "/Filter /Crypt /DecodeParms << /Name /Other >>",
                "/Filter [/FlateDecode /Crypt] /DecodeParms [null << /Name /StdCF >>]",
            ] {
                let page = page("[4 0 R 5 0 R]");
                let file = aes128(&[
                    CATALOG,
                    TREE,
                    Part::Plain(&page),
                    Part::Stream {
                        dict: "",
                        data: RED,
                        encrypt: true,
                    },
                    Part::Stream {
                        dict,
                        data: BLUE,
                        encrypt: true,
                    },
                ]);
                assert_eq!(read(file), joined(&[RED, b""]), "{dict}");
            }
        }
    }
}
