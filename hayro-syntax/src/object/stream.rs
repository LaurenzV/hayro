//! Streams.

use crate::crypto::DecryptionTarget;
use crate::filter::Filter;
use crate::object;
use crate::object::Dict;
use crate::object::Name;
use crate::object::dict::keys::{DECODE_PARMS, DP, F, FILTER, LENGTH, TYPE};
use crate::object::{Array, ObjectIdentifier};
use crate::object::{Object, ObjectLike, ObjectRefLike};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use crate::trivia::is_white_space_character;
use crate::util::find_needle;
use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt::{Debug, Display, Formatter};
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
    /// Stream filters will not be applied.
    pub fn raw_data(&self) -> Cow<'a, [u8]> {
        let ctx = self.dict.ctx();

        if ctx.xref().needs_decryption(ctx)
            && self
                .dict
                .get::<object::String<'_>>(TYPE)
                .map(|t| t.as_ref() != b"XRef")
                .unwrap_or(true)
        {
            match self.dict.obj_id() {
                Some(obj_id) => Cow::Owned(
                    ctx.xref()
                        .decrypt(obj_id, self.data, DecryptionTarget::Stream)
                        // TODO: MAybe an error would be better?
                        .unwrap_or_default(),
                ),
                // Should not be reachable, as streams always have IDs.
                None => Cow::Borrowed(self.data),
            }
        } else {
            Cow::Borrowed(self.data)
        }
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

    /// Return the decoded stream data, or [`DecodeFailure::LimitExceeded`] if any
    /// filter output exceeds `limit`, including data before prediction.
    ///
    /// `ASCIIHexDecode`, `ASCII85Decode`, `LZWDecode`, `FlateDecode` and
    /// `RunLengthDecode` stop shortly after exceeding the limit. Predictor row
    /// buffers are bounded by `limit`, even with no complete input rows. Image
    /// filters decode fully before checking their output. Unfiltered streams are
    /// checked against their raw length.
    ///
    /// The result is not cached.
    pub fn decoded_with_limit(&self, limit: usize) -> Result<Cow<'a, [u8]>, DecodeFailure> {
        self.decode(&ImageDecodeParams::default(), limit)
            .map(|r| r.data)
    }

    /// Return the decoded data of the stream, and return image metadata
    /// if available.
    pub fn decoded_image(
        &self,
        image_params: &ImageDecodeParams,
    ) -> Result<FilterResult<'a>, DecodeFailure> {
        self.decode(image_params, usize::MAX)
    }

    fn decode(
        &self,
        image_params: &ImageDecodeParams,
        limit: usize,
    ) -> Result<FilterResult<'a>, DecodeFailure> {
        let data = self.raw_data();
        let filters_and_params = self.filters_and_params();

        let mut current: Option<FilterResult<'a>> = None;

        for (filter, params) in filters_and_params
            .filters
            .iter()
            .zip(filters_and_params.params.iter())
        {
            let new = filter.apply(
                current.as_ref().map(|c| c.data.as_ref()).unwrap_or(&data),
                params,
                image_params,
                limit,
            )?;
            current = Some(new);
        }

        if current.is_none() && data.len() > limit {
            return Err(DecodeFailure::LimitExceeded);
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

        None
    }
}

impl<'a> Readable<'a> for Stream<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let dict = r.read_with_context::<Dict<'_>>(ctx)?;

        if dict.contains_key(F) {
            return None;
        }

        let offset = r.offset();
        parse_proper(r, &dict).or_else(|| {
            r.jump(offset);
            parse_fallback(r, &dict)
        })
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
    /// The decoded data would hold more bytes than the given limit.
    LimitExceeded,
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
    use crate::object::stream::DecodeFailure;
    use crate::reader::Reader;
    use crate::reader::{ReaderContext, ReaderExt};

    /// 1 GiB of zeros, compressed twice with zlib, in 1,799 bytes:
    ///
    /// ```python
    /// c = zlib.compressobj(9)
    /// inner = b"".join(c.compress(bytes(1 << 20)) for _ in range(1024)) + c.flush()
    /// outer = zlib.compress(inner, 9)
    /// ```
    const FLATE_BOMB: &[u8] = include_bytes!("../../assets/flate_bomb.bin");

    fn stream_bytes(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut bytes = format!("<< /Length {} {dict} >> stream\n", data.len()).into_bytes();
        bytes.extend_from_slice(data);
        bytes.extend_from_slice(b"\nendstream");
        bytes
    }

    fn read_stream(bytes: &[u8]) -> Stream<'_> {
        Reader::new(bytes)
            .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
            .unwrap()
    }

    /// A raw deflate stream that stores `data` in one uncompressed block.
    fn stored_deflate(data: &[u8]) -> Vec<u8> {
        let len = u16::try_from(data.len()).unwrap();
        let mut out = vec![0x01];
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn decoded_with_limit() {
        let bytes = stream_bytes("/Filter /ASCIIHexDecode", b"616263>");
        let stream = read_stream(&bytes);

        assert_eq!(stream.decoded_with_limit(3).unwrap().as_ref(), b"abc");
        assert!(matches!(
            stream.decoded_with_limit(2),
            Err(DecodeFailure::LimitExceeded)
        ));
        assert_eq!(stream.decoded().unwrap().as_ref(), b"abc");
    }

    #[test]
    fn decoded_with_limit_without_filters() {
        let bytes = stream_bytes("", b"abcdefghij");
        let stream = read_stream(&bytes);

        assert_eq!(
            stream.decoded_with_limit(10).unwrap().as_ref(),
            b"abcdefghij"
        );
        assert!(matches!(
            stream.decoded_with_limit(9),
            Err(DecodeFailure::LimitExceeded)
        ));
    }

    #[test]
    fn decoded_with_limit_applies_to_every_filter() {
        // The intermediate output has eight bytes; the final output has four.
        let bytes = stream_bytes(
            "/Filter [/FlateDecode /ASCIIHexDecode]",
            &stored_deflate(b"61626364"),
        );
        let stream = read_stream(&bytes);

        assert_eq!(stream.decoded_with_limit(8).unwrap().as_ref(), b"abcd");
        assert!(matches!(
            stream.decoded_with_limit(7),
            Err(DecodeFailure::LimitExceeded)
        ));
    }

    #[test]
    fn decoded_with_limit_with_png_predictor() {
        let bytes = stream_bytes(
            "/Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 3 >>",
            &stored_deflate(&[2, 1, 2, 3, 2, 1, 1, 1]),
        );
        let stream = read_stream(&bytes);

        assert_eq!(
            stream.decoded_with_limit(8).unwrap().as_ref(),
            &[1, 2, 3, 2, 3, 4]
        );
        assert_eq!(stream.decoded().unwrap().as_ref(), &[1, 2, 3, 2, 3, 4]);
        assert!(matches!(
            stream.decoded_with_limit(7),
            Err(DecodeFailure::LimitExceeded)
        ));
    }

    #[test]
    fn decoded_with_limit_bounds_predictor_rows() {
        let flate = stored_deflate(&[]);
        let lzw = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        for (filter, data) in [("FlateDecode", flate.as_slice()), ("LZWDecode", &lzw[..])] {
            for predictor in [2, 12] {
                // The row buffer needs 16 MiB even with empty input.
                let bytes = stream_bytes(
                    &format!(
                        "/Filter /{filter} /DecodeParms << /Predictor {predictor} \
                         /BitsPerComponent 1 /Colors 1 /Columns 134217728 >>"
                    ),
                    data,
                );
                assert!(matches!(
                    read_stream(&bytes).decoded_with_limit(1024),
                    Err(DecodeFailure::LimitExceeded)
                ));
            }
        }
    }

    #[test]
    fn decoded_with_limit_with_tiff_predictor() {
        // Eight one-bit samples: horizontal differencing is addition modulo two.
        let bytes = stream_bytes(
            "/Filter /FlateDecode /DecodeParms << /Predictor 2 \
             /BitsPerComponent 1 /Colors 1 /Columns 8 >>",
            &stored_deflate(&[0b1111_0000]),
        );
        let stream = read_stream(&bytes);
        assert_eq!(
            stream.decoded_with_limit(1).unwrap().as_ref(),
            &[0b1010_0000]
        );
        assert!(matches!(
            stream.decoded_with_limit(0),
            Err(DecodeFailure::LimitExceeded)
        ));
    }

    #[test]
    fn decoded_with_limit_stops_a_flate_bomb() {
        let bytes = stream_bytes("/Filter [/FlateDecode /FlateDecode]", FLATE_BOMB);
        let stream = read_stream(&bytes);

        // Exceed the limit in the first filter, then in the second.
        assert!(matches!(
            stream.decoded_with_limit(64 * 1024),
            Err(DecodeFailure::LimitExceeded)
        ));
        assert!(matches!(
            stream.decoded_with_limit(4 * 1024 * 1024),
            Err(DecodeFailure::LimitExceeded)
        ));
    }

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
}
