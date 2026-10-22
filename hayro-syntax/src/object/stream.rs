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
use crate::util::{OptionLog, find_needle};
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
                parse_fallback(r, &dict, ctx)
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

fn parse_fallback<'a>(
    r: &mut Reader<'a>,
    dict: &Dict<'a>,
    ctx: &ReaderContext<'a>,
) -> Option<Stream<'a>> {
    // Like `Object::read`, only treat a dictionary followed by `stream` as a stream, so
    // that a dictionary doesn't pick up the data of a later object.
    r.skip_white_spaces_and_comments();
    r.forward_tag(b"stream")?;

    r.forward_tag(b"\n")
        .or_else(|| r.forward_tag(b"\r\n"))
        // Technically not allowed, but no reason to not try it.
        .or_else(|| r.forward_tag(b"\r"))?;

    let start = r.offset();
    let (endstream_offset, data_end) = match ctx.xref().endstreams(r.data) {
        Some(endstreams) => endstreams.after(start)?,
        None => endstream_after(r.data, start)?,
    };
    let data = r.data.get(start..data_end)?;
    r.jump(endstream_offset + b"endstream".len());

    Some(Stream::new(data, dict.clone()))
}

/// The offset of the first `endstream` at or after `start` in `data`, and where the data
/// from `start` up to it end, without the white space before it.
fn endstream_after(data: &[u8], start: usize) -> Option<(usize, usize)> {
    let tail = data.get(start..)?;
    let offset = searched(tail, b"endstream")?;

    Some((
        start + offset,
        start + trim_trailing_ascii_whitespace(&tail[..offset]),
    ))
}

/// Where each `endstream` of a file is, found once for the whole file.
///
/// The data of a stream with a wrong /Length end at the first `endstream` after them.
/// Searching the file for it on every read would take time quadratic in the file size
/// when many streams have no `endstream` (or one stream is read many times) and the
/// search runs on far beyond the stream, so the streams of a document are looked up here
/// instead.
#[derive(Debug, Default)]
pub(crate) struct Endstreams(Vec<(usize, usize)>);

impl Endstreams {
    pub(crate) fn new(data: &[u8]) -> Self {
        let mut endstreams = Vec::new();
        let mut offset = 0;

        while let Some(found) = searched(&data[offset..], b"endstream") {
            let endstream_offset = offset + found;
            endstreams.push((
                endstream_offset,
                trim_trailing_ascii_whitespace(&data[..endstream_offset]),
            ));
            offset = endstream_offset + b"endstream".len();
        }

        Self(endstreams)
    }

    /// The same as `endstream_after` for the file this was built from.
    fn after(&self, start: usize) -> Option<(usize, usize)> {
        let index = self.0.partition_point(|(offset, _)| *offset < start);
        let (endstream_offset, data_end) = self.0.get(index)?;

        Some((*endstream_offset, (*data_end).max(start)))
    }
}

fn searched(data: &[u8], needle: &[u8]) -> Option<usize> {
    let found = find_needle(data, needle);
    #[cfg(test)]
    SEARCHED.set(SEARCHED.get() + found.unwrap_or(data.len()));
    found
}

#[cfg(test)]
std::thread_local! {
    /// The bytes searched for `endstream` on this thread, which the tests bound to show
    /// that the streams of a file are read in linear time.
    static SEARCHED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
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
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;
    use alloc::{format, vec};

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
    fn stream_fallback_without_endstream() {
        // Without its own `endstream`, a stream's data run to the next one.
        let data =
            b"<< /Length 1 >> stream\nabc\nendobj\n2 0 obj\n<< /Length 1 >>\nstream\nx\nendstream";
        let mut r = Reader::new(data);
        let stream = r
            .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(
            stream.data,
            b"abc\nendobj\n2 0 obj\n<< /Length 1 >>\nstream\nx"
        );
        assert!(r.at_end());
    }

    #[test]
    fn stream_fallback_endobj_in_data() {
        let data = b"<< /Length 1 >> stream\n(endobj) abc\nendstream\nendobj";
        let mut r = Reader::new(data);
        let stream = r
            .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(stream.data, b"(endobj) abc");
    }

    #[test]
    fn stream_fallback_dict_without_stream() {
        let data = b"<< /Foo 1 >>\nendobj\n2 0 obj\n<< /Length 5 >>\nstream\nhello\nendstream";
        let mut r = Reader::new(data);

        assert!(
            r.read_with_context::<Stream<'_>>(&ReaderContext::dummy())
                .is_none()
        );
    }

    // Looking up `endstream` in the index finds what searching for it does, from every
    // offset of files made of the pieces that matter to it.
    #[test]
    fn endstreams_match_search() {
        let pieces: [&[u8]; 6] = [b"endstream", b"endstrea", b" ", b"\r\n", b"x", b"m"];
        let mut seed = 1_u32;

        for _ in 0..2000 {
            let mut data = Vec::new();
            for _ in 0..seed % 12 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                data.extend_from_slice(pieces[seed as usize % pieces.len()]);
            }

            let endstreams = super::Endstreams::new(&data);
            for start in 0..=data.len() {
                assert_eq!(
                    endstreams.after(start),
                    super::endstream_after(&data, start),
                    "{:?} from {start}",
                    alloc::string::String::from_utf8_lossy(&data)
                );
            }
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Malformed {
        /// Each page's content stream has a wrong /Length and no `endstream`.
        NoEndstream,
        /// As `NoEndstream`, and no object ends with `endobj`.
        NoEndstreamNoEndobj,
        /// As `NoEndstream`, with a comment inside each object header.
        NoEndstreamCommentInHeader,
        /// All pages share one content stream with a wrong /Length, whose object is
        /// followed by a comment as long as the rest of the file.
        SharedWithLongComment,
    }

    /// A file with `n` pages whose content streams are malformed as `malformed`, the
    /// bytes searched for `endstream` while reading the content of every page, and the
    /// content of each page.
    fn read_malformed(n: usize, malformed: Malformed) -> (usize, Vec<Option<Vec<u8>>>) {
        let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")),
        ];
        for i in 0..n {
            let contents = match malformed {
                Malformed::SharedWithLongComment => 4,
                _ => 4 + 2 * i,
            };
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents {contents} 0 R >>"
            ));
            objects.push(match malformed {
                Malformed::SharedWithLongComment if i == 0 => {
                    "<< /Length 1 >>\nstream\n0 0 m 100 100 l S\nendstream".to_string()
                }
                Malformed::SharedWithLongComment => "null".to_string(),
                _ => "<< /Length 1 >>\nstream\n0 0 m 100 100 l S".to_string(),
            });
        }

        let mut file = b"%PDF-1.7\n".to_vec();
        let mut offsets = vec![0; objects.len()];
        // Written last to first, so that another object follows each content stream.
        for (i, object) in objects.iter().enumerate().rev() {
            offsets[i] = file.len();
            let number = i + 1;
            file.extend(
                match malformed {
                    Malformed::NoEndstreamNoEndobj => format!("{number} 0 obj\n{object}\n"),
                    Malformed::NoEndstreamCommentInHeader => {
                        format!("{number} % c\n0 obj\n{object}\nendobj\n")
                    }
                    _ => format!("{number} 0 obj\n{object}\nendobj\n"),
                }
                .bytes(),
            );
            if matches!(malformed, Malformed::SharedWithLongComment) && number == 4 {
                file.extend(format!("%{}\n", "x".repeat(100 * n)).bytes());
            }
        }
        let xref = file.len();
        file.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            file.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        file.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );

        super::SEARCHED.set(0);
        let pdf = crate::Pdf::new(file).unwrap();
        let pages = pdf.pages();
        assert_eq!(pages.len(), n, "{malformed:?}");
        let contents = pages
            .iter()
            .map(|page| page.page_stream().map(<[u8]>::to_vec))
            .collect();

        (super::SEARCHED.get(), contents)
    }

    // Each page reads what it read before the index of `endstream` existed, and doubling
    // the pages at most doubles the bytes searched.
    #[test]
    fn malformed_streams_load_in_linear_time() {
        for malformed in [
            Malformed::NoEndstream,
            Malformed::NoEndstreamNoEndobj,
            Malformed::NoEndstreamCommentInHeader,
            Malformed::SharedWithLongComment,
        ] {
            let [(small, small_contents), (large, large_contents)] =
                [500, 1000].map(|n| read_malformed(n, malformed));
            assert!(
                large < small * 21 / 10 + 100,
                "{malformed:?}: bytes searched {small} for 500 pages, {large} for 1000"
            );

            let expected = match malformed {
                Malformed::SharedWithLongComment => Some(b"0 0 m 100 100 l S".to_vec()),
                _ => None,
            };
            for contents in [small_contents, large_contents] {
                assert!(
                    contents.iter().all(|content| *content == expected),
                    "{malformed:?}"
                );
            }
        }
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
