//! Streams.

use crate::crypto::DecryptionTarget;
use crate::filter::Filter;
use crate::object;
use crate::object::Dict;
use crate::object::Name;
use crate::object::dict::keys::{DECODE_PARMS, DP, F, FILTER, LENGTH, TYPE};
use crate::object::{Array, ObjectIdentifier};
use crate::object::{Object, ObjectLike};
use crate::reader::{Readable, Reader, ReaderContext, Skippable};
use crate::util::OptionLog;
use log::{info, warn};
use std::borrow::Cow;
use std::fmt::{Debug, Formatter};

/// A stream of arbitrary data.
#[derive(Clone, PartialEq)]
pub struct Stream<'a> {
    dict: Dict<'a>,
    data: &'a [u8],
}

/// Additional parameters for decoding images.
#[derive(Clone, PartialEq, Default)]
pub struct ImageDecodeParams {
    /// Whether the color space of the image is an indexed color space.
    pub is_indexed: bool,
    /// The bits per component of the image, if that information is available.
    pub bpc: Option<u8>,
}

impl<'a> Stream<'a> {
    /// Return the raw, decrypted data of the stream.
    ///
    /// Stream filters will not be applied.
    pub fn raw_data(&self) -> Cow<'a, [u8]> {
        let ctx = self.dict.ctx();

        if ctx.xref.needs_decryption(ctx)
            && self
                .dict
                .get::<object::String>(TYPE)
                .map(|t| t.get().as_ref() != b"XRef")
                .unwrap_or(true)
        {
            Cow::Owned(
                ctx.xref
                    .decrypt(
                        self.dict.obj_id().unwrap(),
                        self.data,
                        DecryptionTarget::Stream,
                    )
                    // TODO: MAybe an error would be better?
                    .unwrap_or_default(),
            )
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
        self.dict.obj_id().unwrap()
    }

    /// Return the decoded data of the stream.
    ///
    /// Note that the result of this method will not be cached, so calling it multiple
    /// times is expensive.
    pub fn decoded(&self) -> Result<Vec<u8>, DecodeFailure> {
        self.decoded_image(&ImageDecodeParams::default())
            .map(|r| r.data)
    }

    /// Return the decoded data of the stream, and return image metadata
    /// if available.
    pub fn decoded_image(
        &self,
        image_params: &ImageDecodeParams,
    ) -> Result<FilterResult, DecodeFailure> {
        let data = self.raw_data();

        if let Some(filter) = self
            .dict
            .get::<Name>(F)
            .or_else(|| self.dict.get::<Name>(FILTER))
            .and_then(|n| Filter::from_name(n))
        {
            let params = self
                .dict
                .get::<Dict>(DP)
                .or_else(|| self.dict.get::<Dict>(DECODE_PARMS));

            filter.apply(&data, params.clone().unwrap_or_default(), image_params)
        } else if let Some(filters) = self
            .dict
            .get::<Array>(F)
            .or_else(|| self.dict.get::<Array>(FILTER))
        {
            let filters = filters
                .iter::<Name>()
                .map(|n| Filter::from_name(n))
                .collect::<Option<Vec<_>>>()
                .ok_or(DecodeFailure::Unknown)?;
            let params: Vec<_> = self
                .dict
                .get::<Array>(DP)
                .or_else(|| self.dict.get::<Array>(DECODE_PARMS))
                .map(|a| a.iter::<Object>().collect())
                .unwrap_or_default();

            let mut current: Option<FilterResult> = None;

            for (i, filter) in filters.iter().enumerate() {
                let params = params.get(i).and_then(|p| p.clone().cast::<Dict>());

                let new = filter.apply(
                    current.as_ref().map(|c| c.data.as_ref()).unwrap_or(&data),
                    params.clone().unwrap_or_default(),
                    image_params,
                )?;
                current = Some(new);
            }

            Ok(current.unwrap_or(FilterResult {
                data: data.to_vec(),
                image_data: None,
            }))
        } else {
            Ok(FilterResult {
                data: data.to_vec(),
                image_data: None,
            })
        }
    }

    pub(crate) fn from_raw(data: &'a [u8], dict: Dict<'a>) -> Self {
        Self { dict, data }
    }
}

impl Debug for Stream<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Stream (len: {:?})", self.data.len())
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
        let dict = r.read_with_context::<Dict>(ctx)?;

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
    /// A JPEG2000 image was encountered, while the `jpeg2000` feature was disabled.
    JpxImage,
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
}

/// Additional data that is extracted from some image streams.
pub struct ImageData {
    /// An optional alpha channel of the image.
    pub alpha: Option<Vec<u8>>,
    /// The color space of the image.
    pub color_space: ImageColorSpace,
    /// The bits per component of the image.
    pub bits_per_component: u8,
}

/// The result of applying a filter.
pub struct FilterResult {
    /// The decoded data.
    pub data: Vec<u8>,
    /// Additional data that is extracted from JPX image streams.
    pub image_data: Option<ImageData>,
}

impl FilterResult {
    pub(crate) fn from_data(data: Vec<u8>) -> Self {
        Self {
            data,
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

    Some(Stream {
        data,
        dict: dict.clone(),
    })
}

fn parse_fallback<'a>(r: &mut Reader<'a>, dict: &Dict<'a>) -> Option<Stream<'a>> {
    while r.forward_tag(b"stream").is_none() {
        r.read_byte()?;
    }

    r.forward_tag(b"\n").or_else(|| r.forward_tag(b"\r\n"))?;

    let data_start = r.tail()?;
    let start = r.offset();

    loop {
        if r.peek_byte()?.is_ascii_whitespace() || r.peek_tag(b"endstream").is_some() {
            let length = r.offset() - start;
            let data = data_start.get(..length)?;

            r.skip_white_spaces();

            // This was just a whitespace in the data stream but not actually marking the end
            // of the stream, so continue searching.
            if r.forward_tag(b"endstream").is_none() {
                continue;
            }

            let stream = Stream {
                data,
                dict: dict.clone(),
            };

            // Try decoding the stream to see if it is valid.
            if stream.decoded().is_ok() {
                info!("managed to reconstruct the stream");

                // Seems like we found the end!
                return Some(stream);
            }
        } else {
            r.read_byte()?;
        }
    }
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

#[cfg(test)]
mod tests {
    use crate::object::Stream;
    use crate::reader::{Reader, ReaderContext};

    #[test]
    fn stream() {
        let data = b"<< /Length 10 >> stream\nabcdefghij\nendstream";
        let mut r = Reader::new(data);
        let stream = r
            .read_with_context::<Stream>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(stream.data, b"abcdefghij");
    }
}
