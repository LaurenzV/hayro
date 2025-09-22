//! Reading and querying the xref table of a PDF file.

use crate::crypto::{DecryptionError, DecryptionTarget, Decryptor, get};
use crate::data::Data;
use crate::object::Dict;
use crate::object::Name;
use crate::object::ObjectIdentifier;
use crate::object::Stream;
use crate::object::dict::keys::{
    ENCRYPT, FIRST, ID, INDEX, N, PAGES, PREV, ROOT, SIZE, TYPE, VERSION, W, XREF_STM,
};
use crate::object::indirect::IndirectObject;
use crate::object::{Array, MaybeRef};
use crate::object::{Object, ObjectLike};
use crate::pdf::PdfVersion;
use crate::reader::{Readable, Reader, ReaderContext};
use crate::{PdfData, object};
use log::{error, warn};
use rustc_hash::FxHashMap;
use std::cmp::max;
use std::iter;
use std::ops::Deref;
use std::sync::{Arc, RwLock};

pub(crate) const XREF_ENTRY_LEN: usize = 20;

#[derive(Debug, Copy, Clone)]
pub(crate) enum XRefError {
    Unknown,
    Encryption(DecryptionError),
}

/// Parse the "root" xref from the PDF.
pub(crate) fn root_xref(data: PdfData) -> Result<XRef, XRefError> {
    let mut xref_map = FxHashMap::default();
    let xref_pos = find_last_xref_pos(data.as_ref().as_ref()).ok_or(XRefError::Unknown)?;
    let trailer = populate_xref_impl(data.as_ref().as_ref(), xref_pos, &mut xref_map)
        .ok_or(XRefError::Unknown)?;

    XRef::new(data.clone(), xref_map, trailer, false)
}

/// Try to manually parse the PDF to build an xref table and trailer dictionary.
pub(crate) fn fallback(data: PdfData) -> Option<XRef> {
    warn!("xref table was invalid, trying to manually build xref table");
    let (xref_map, trailer_dict) = fallback_xref_map(data.as_ref().as_ref());

    if let Some(trailer_dict_data) = trailer_dict {
        warn!("rebuild xref table with {} entries", xref_map.len());

        XRef::new(data.clone(), xref_map, trailer_dict_data, true).ok()
    } else {
        warn!("couldn't find trailer dictionary, failed to rebuild xref table");

        None
    }
}

fn fallback_xref_map(data: &[u8]) -> (XrefMap, Option<&[u8]>) {
    let mut xref_map = FxHashMap::default();
    let mut trailer_dicts = vec![];

    let mut r = Reader::new(data);

    let mut dummy_ctx = ReaderContext::dummy();
    let mut last_obj_num = None;

    loop {
        let cur_pos = r.offset();

        let mut old_r = r.clone();

        if let Some(obj_id) = r.read::<ObjectIdentifier>(&dummy_ctx) {
            let mut cloned = r.clone();
            // Check that the object following it is actually valid before inserting it.
            cloned.skip_white_spaces_and_comments();
            if cloned.skip::<Object>(false).is_some() {
                xref_map.insert(obj_id, EntryType::Normal(cur_pos));
                last_obj_num = Some(obj_id);
                dummy_ctx.obj_number = Some(obj_id);
            }
        } else if let Some(dict) = r.read::<Dict>(&dummy_ctx) {
            if dict.contains_key(ROOT) {
                trailer_dicts.push(dict);
            }

            if let Some(stream) = old_r.read::<Stream>(&dummy_ctx)
                && stream.dict().get::<Name>(TYPE).as_deref() == Some(b"ObjStm")
                && let Some(data) = stream.decoded().ok()
                && let Some(last_obj_num) = last_obj_num
                && let Some(obj_stream) = ObjectStream::new(stream, &data, &dummy_ctx)
            {
                for (idx, (obj_num, _)) in obj_stream.offsets.iter().enumerate() {
                    let id = ObjectIdentifier::new(*obj_num as i32, 0);
                    xref_map.insert(
                        id,
                        EntryType::ObjStream(last_obj_num.obj_num as u32, idx as u32),
                    );
                }
            }
        } else {
            r.read_byte();
        }

        if r.at_end() {
            break;
        }
    }

    // Try to choose the right trailer dict by doing basic validation.
    let mut trailer_dict = None;

    for dict in trailer_dicts {
        if let Some(root_id) = dict.get_raw::<Dict>(ROOT) {
            let check = |dict: &Dict| -> bool { dict.contains_key(PAGES) };

            match root_id {
                MaybeRef::Ref(r) => {
                    if let Some(EntryType::Normal(offset)) = xref_map.get(&r.into()) {
                        let mut reader = Reader::new(&data[*offset..]);

                        if let Some(obj) =
                            reader.read_with_context::<IndirectObject<Dict>>(&dummy_ctx)
                            && check(&obj.clone().get())
                        {
                            trailer_dict = Some(dict);
                        }
                    }
                }
                MaybeRef::NotRef(d) => {
                    if check(&d) {
                        trailer_dict = Some(dict);
                    }
                }
            }
        }
    }

    (xref_map, trailer_dict.map(|d| d.data()))
}

static DUMMY_XREF: &XRef = &XRef(Inner::Dummy);

/// An xref table.
#[derive(Debug, Clone)]
pub struct XRef(Inner);

impl XRef {
    fn new(
        data: PdfData,
        xref_map: XrefMap,
        trailer_dict_data: &[u8],
        repaired: bool,
    ) -> Result<Self, XRefError> {
        // This is a bit hacky, but the problem is we can't read the resolved trailer dictionary
        // before we actually created the xref struct. So we first create it using dummy data
        // and then populate the data.
        let trailer_data = TrailerData::dummy();

        let mut xref = Self(Inner::Some(Arc::new(SomeRepr {
            data: Arc::new(Data::new(data)),
            map: Arc::new(RwLock::new(MapRepr { xref_map, repaired })),
            decryptor: Arc::new(Decryptor::None),
            trailer_data,
        })));

        let mut r = Reader::new(trailer_dict_data);
        let trailer_dict = r
            .read_with_context::<Dict>(&ReaderContext::new(&xref, false))
            .ok_or(XRefError::Unknown)?;

        let decryptor = if let Some(encryption_dict) = trailer_dict.get::<Dict>(ENCRYPT) {
            let Some(id) = trailer_dict
                .get::<Array>(ID)
                .and_then(|a| a.flex_iter().next::<object::String>())
            else {
                return Err(XRefError::Encryption(DecryptionError::MissingIDEntry));
            };

            get(&encryption_dict, id.get().as_ref()).map_err(XRefError::Encryption)?
        } else {
            Decryptor::None
        };

        let root = trailer_dict.get::<Dict>(ROOT).ok_or(XRefError::Unknown)?;
        let pages_ref = root.get_ref(PAGES).ok_or(XRefError::Unknown)?;
        let version = root
            .get::<Name>(VERSION)
            .and_then(|v| PdfVersion::from_bytes(v.deref()));

        let td = TrailerData {
            pages_ref: pages_ref.into(),
            version,
        };

        match &mut xref.0 {
            Inner::Dummy => unreachable!(),
            Inner::Some(r) => {
                Arc::make_mut(r).trailer_data = td;
                Arc::make_mut(r).decryptor = Arc::new(decryptor);
            }
        }

        Ok(xref)
    }

    fn is_repaired(&self) -> bool {
        match &self.0 {
            Inner::Dummy => false,
            Inner::Some(r) => {
                let locked = r.map.read().unwrap();
                locked.repaired
            }
        }
    }

    pub(crate) fn dummy() -> &'static XRef {
        DUMMY_XREF
    }

    pub(crate) fn len(&self) -> usize {
        match &self.0 {
            Inner::Dummy => 0,
            Inner::Some(r) => r.map.read().unwrap().xref_map.len(),
        }
    }

    pub(crate) fn trailer_data(&self) -> &TrailerData {
        match &self.0 {
            Inner::Dummy => unreachable!(),
            Inner::Some(r) => &r.trailer_data,
        }
    }

    pub(crate) fn objects(&self) -> impl IntoIterator<Item = Object<'_>> + '_ {
        match &self.0 {
            Inner::Dummy => unimplemented!(),
            Inner::Some(r) => iter::from_fn(move || {
                let locked = r.map.read().unwrap();
                let mut iter = locked.xref_map.keys();

                iter.next()
                    .and_then(|k| self.get_with(*k, &ReaderContext::new(self, false)))
            }),
        }
    }

    pub(crate) fn repair(&self) {
        let Inner::Some(r) = &self.0 else {
            unreachable!();
        };

        let mut locked = r.map.try_write().unwrap();
        assert!(!locked.repaired);

        let (xref_map, _) = fallback_xref_map(r.data.get());
        locked.xref_map = xref_map;
        locked.repaired = true;
    }

    #[inline]
    pub(crate) fn needs_decryption(&self, ctx: &ReaderContext) -> bool {
        match &self.0 {
            Inner::Dummy => false,
            Inner::Some(r) => {
                if matches!(r.decryptor.as_ref(), Decryptor::None) {
                    false
                } else {
                    !ctx.in_content_stream
                }
            }
        }
    }

    #[inline]
    pub(crate) fn decrypt(
        &self,
        id: ObjectIdentifier,
        data: &[u8],
        target: DecryptionTarget,
    ) -> Option<Vec<u8>> {
        match &self.0 {
            Inner::Dummy => Some(data.to_vec()),
            Inner::Some(r) => r.decryptor.decrypt(id, data, target),
        }
    }

    /// Return the object with the given identifier.
    #[allow(private_bounds)]
    pub fn get<'a, T>(&'a self, id: ObjectIdentifier) -> Option<T>
    where
        T: ObjectLike<'a>,
    {
        let ctx = ReaderContext::new(self, false);
        self.get_with(id, &ctx)
    }

    /// Return the object with the given identifier.
    #[allow(private_bounds)]
    pub(crate) fn get_with<'a, T>(
        &'a self,
        id: ObjectIdentifier,
        ctx: &ReaderContext<'a>,
    ) -> Option<T>
    where
        T: ObjectLike<'a>,
    {
        let Inner::Some(repr) = &self.0 else {
            return None;
        };

        let locked = repr.map.try_read().unwrap();

        let mut r = Reader::new(repr.data.get());

        let entry = *locked.xref_map.get(&id).or({
            // An indirect reference to an undefined object shall not be considered an error by a PDF processor; it
            // shall be treated as a reference to the null object.
            None
        })?;
        drop(locked);

        let mut ctx = ctx.clone();
        ctx.in_content_stream = false;

        match entry {
            EntryType::Normal(offset) => {
                r.jump(offset);

                if let Some(object) = r.read_with_context::<IndirectObject<T>>(&ctx) {
                    if object.id() == &id {
                        return Some(object.get());
                    }
                } else {
                    // There is a valid object at the offset, it's just not of the type the caller
                    // expected, which is fine.
                    if r.skip_not_in_content_stream::<IndirectObject<Object>>()
                        .is_some()
                    {
                        return None;
                    }
                };

                // The xref table is broken, try to repair if not already repaired.
                if self.is_repaired() {
                    error!(
                        "attempt was made at repairing xref, but object {id:?} still couldn't be read"
                    );

                    None
                } else {
                    warn!("broken xref, attempting to repair");

                    self.repair();

                    // Now try reading again.
                    self.get_with::<T>(id, &ctx)
                }
            }
            EntryType::ObjStream(obj_stram_gen_num, index) => {
                // Generation number is implicitly 0.
                let obj_stream_id = ObjectIdentifier::new(obj_stram_gen_num as i32, 0);

                let stream = self.get_with::<Stream>(obj_stream_id, &ctx)?;
                let data = repr.data.get_with(obj_stream_id, &ctx)?;
                let object_stream = ObjectStream::new(stream, data, &ctx)?;
                object_stream.get(index)
            }
        }
    }
}

pub(crate) fn find_last_xref_pos(data: &[u8]) -> Option<usize> {
    let mut finder = Reader::new(data);
    let mut pos = finder.len().checked_sub(1)?;
    finder.jump(pos);

    let needle = b"startxref";

    loop {
        if finder.forward_tag(needle).is_some() {
            finder.skip_white_spaces_and_comments();

            let offset = finder.read_without_context::<i32>()?.try_into().ok()?;

            return Some(offset);
        }

        pos = pos.checked_sub(1)?;
        finder.jump(pos);
    }
}

/// A type of xref entry.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum EntryType {
    /// An indirect object that is at a specific offset in the original data.
    Normal(usize),
    /// An indirect object that is part of an object stream. First number indicates the object
    /// number of the _object stream_ (the generation number is always 0), the second number indicates
    /// the index in the object stream.
    ObjStream(u32, u32),
}

type XrefMap = FxHashMap<ObjectIdentifier, EntryType>;

/// Representation of a proper xref table.
#[derive(Debug)]
struct MapRepr {
    xref_map: XrefMap,
    repaired: bool,
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct TrailerData {
    pub pages_ref: ObjectIdentifier,
    pub version: Option<PdfVersion>,
}

impl TrailerData {
    pub fn dummy() -> Self {
        Self {
            pages_ref: ObjectIdentifier::new(0, 0),
            version: None,
        }
    }
}

#[derive(Debug, Clone)]
struct SomeRepr {
    data: Arc<Data>,
    map: Arc<RwLock<MapRepr>>,
    decryptor: Arc<Decryptor>,
    trailer_data: TrailerData,
}

#[derive(Debug, Clone)]
enum Inner {
    /// A dummy xref table that doesn't have any entries.
    Dummy,
    /// A proper xref table.
    Some(Arc<SomeRepr>),
}

#[derive(Debug)]
struct XRefEntry {
    offset: usize,
    gen_number: i32,
    used: bool,
}

impl XRefEntry {
    pub(crate) fn read(data: &[u8]) -> Option<XRefEntry> {
        #[inline(always)]
        fn parse_u32(data: &[u8]) -> Option<u32> {
            let mut accum = 0;

            for byte in data {
                accum *= 10;

                match *byte {
                    b'0'..=b'9' => accum += (*byte - b'0') as u32,
                    _ => return None,
                }
            }

            Some(accum)
        }

        let offset = parse_u32(&data[0..10])? as usize;
        let gen_number = parse_u32(&data[11..16])? as i32;

        let used = data[17] == b'n';

        Some(Self {
            offset,
            gen_number,
            used,
        })
    }
}

fn populate_xref_impl<'a>(data: &'a [u8], pos: usize, xref_map: &mut XrefMap) -> Option<&'a [u8]> {
    let mut reader = Reader::new(data);
    reader.jump(pos);

    let mut r2 = reader.clone();
    if reader
        .clone()
        .read_without_context::<ObjectIdentifier>()
        .is_some()
    {
        populate_from_xref_stream(data, &mut r2, xref_map)
    } else {
        populate_from_xref_table(data, &mut r2, xref_map)
    }
}

pub(super) struct SubsectionHeader {
    pub(super) start: u32,
    pub(super) num_entries: u32,
}

impl Readable<'_> for SubsectionHeader {
    fn read(r: &mut Reader<'_>, _: &ReaderContext) -> Option<Self> {
        r.skip_white_spaces();
        let start = r.read_without_context::<u32>()?;
        r.skip_white_spaces();
        let num_entries = r.read_without_context::<u32>()?;
        r.skip_white_spaces();

        Some(Self { start, num_entries })
    }
}

/// Populate the xref table, and return the trailer dict.
fn populate_from_xref_table<'a>(
    data: &'a [u8],
    reader: &mut Reader<'a>,
    insert_map: &mut XrefMap,
) -> Option<&'a [u8]> {
    let trailer = {
        let mut reader = reader.clone();
        read_xref_table_trailer(&mut reader, &ReaderContext::dummy())?
    };

    reader.skip_white_spaces();
    reader.forward_tag(b"xref")?;
    reader.skip_white_spaces();

    let mut max_obj = 0;

    if let Some(prev) = trailer.get::<i32>(PREV) {
        // First insert the entries from any previous xref tables.
        populate_xref_impl(data, prev as usize, insert_map)?;
    }

    // In hybrid files, entries in `XRefStm` should have higher priority, therefore we insert them
    // after looking at `PREV`.
    if let Some(xref_stm) = trailer.get::<i32>(XREF_STM) {
        populate_xref_impl(data, xref_stm as usize, insert_map)?;
    }

    while let Some(header) = reader.read_without_context::<SubsectionHeader>() {
        reader.skip_white_spaces();

        let start = header.start;
        let end = start + header.num_entries;

        for obj_number in start..end {
            max_obj = max(max_obj, obj_number);
            let bytes = reader.read_bytes(XREF_ENTRY_LEN)?;
            let entry = XRefEntry::read(bytes)?;

            // Specification says we should ignore any object number > SIZE, but probably
            // not important?
            if entry.used {
                insert_map.insert(
                    ObjectIdentifier::new(obj_number as i32, entry.gen_number),
                    EntryType::Normal(entry.offset),
                );
            }
        }
    }

    Some(trailer.data())
}

fn populate_from_xref_stream<'a>(
    data: &'a [u8],
    reader: &mut Reader<'a>,
    insert_map: &mut XrefMap,
) -> Option<&'a [u8]> {
    let stream = reader
        .read_with_context::<IndirectObject<Stream>>(&ReaderContext::dummy())?
        .get();

    if let Some(prev) = stream.dict().get::<i32>(PREV) {
        // First insert the entries from any previous xref tables.
        let _ = populate_xref_impl(data, prev as usize, insert_map)?;
    }

    let size = stream.dict().get::<u32>(SIZE)?;

    let [f1_len, f2_len, f3_len] = stream.dict().get::<[u8; 3]>(W)?;

    if f2_len > size_of::<u64>() as u8 {
        error!("xref offset length is larger than the allowed limit");

        return None;
    }

    // Do such files exist?
    if f1_len != 1 {
        warn!("first field in xref stream was longer than 1");
    }

    let xref_data = stream.decoded().ok()?;
    let mut xref_reader = Reader::new(xref_data.as_ref());

    if let Some(arr) = stream.dict().get::<Array>(INDEX) {
        let iter = arr.iter::<(u32, u32)>();

        for (start, num_elements) in iter {
            xref_stream_subsection(
                &mut xref_reader,
                start,
                num_elements,
                f1_len,
                f2_len,
                f3_len,
                insert_map,
            )?;
        }
    } else {
        xref_stream_subsection(
            &mut xref_reader,
            0,
            size,
            f1_len,
            f2_len,
            f3_len,
            insert_map,
        )?;
    }

    Some(stream.dict().data())
}

fn xref_stream_num(data: &[u8]) -> Option<u32> {
    Some(match data.len() {
        0 => return None,
        1 => u8::from_be(data[0]) as u32,
        2 => u16::from_be_bytes(data[0..2].try_into().ok()?) as u32,
        3 => u32::from_be_bytes([0, data[0], data[1], data[2]]),
        4 => u32::from_be_bytes(data[0..4].try_into().ok()?),
        8 => {
            if let Ok(num) = u32::try_from(u64::from_be_bytes(data[0..8].try_into().ok()?)) {
                return Some(num);
            } else {
                warn!("xref stream number is too large");

                return None;
            }
        }
        n => {
            warn!("invalid xref stream number {n}");

            return None;
        }
    })
}

fn xref_stream_subsection<'a>(
    xref_reader: &mut Reader<'a>,
    start: u32,
    num_elements: u32,
    f1_len: u8,
    f2_len: u8,
    f3_len: u8,
    insert_map: &mut XrefMap,
) -> Option<()> {
    for i in 0..num_elements {
        let f_type = if f1_len == 0 {
            1
        } else {
            // We assume a length of 1.
            xref_reader.read_bytes(1)?[0]
        };

        let obj_number = start + i;

        match f_type {
            // We don't care about free objects.
            0 => {
                xref_reader.skip_bytes(f2_len as usize + f3_len as usize)?;
            }
            1 => {
                let offset = if f2_len > 0 {
                    let data = xref_reader.read_bytes(f2_len as usize)?;
                    xref_stream_num(data)?
                } else {
                    0
                };

                let gen_number = if f3_len > 0 {
                    let data = xref_reader.read_bytes(f3_len as usize)?;
                    xref_stream_num(data)?
                } else {
                    0
                };

                insert_map.insert(
                    ObjectIdentifier::new(obj_number as i32, gen_number as i32),
                    EntryType::Normal(offset as usize),
                );
            }
            2 => {
                let obj_stream_number = {
                    let data = xref_reader.read_bytes(f2_len as usize)?;
                    xref_stream_num(data)?
                };
                let gen_number = 0;
                let index = if f3_len > 0 {
                    let data = xref_reader.read_bytes(f3_len as usize)?;
                    xref_stream_num(data)?
                } else {
                    0
                };

                insert_map.insert(
                    ObjectIdentifier::new(obj_number as i32, gen_number),
                    EntryType::ObjStream(obj_stream_number, index),
                );
            }
            _ => {
                warn!("xref has unknown field type {f_type}");

                return None;
            }
        }
    }

    Some(())
}

fn read_xref_table_trailer<'a>(
    reader: &mut Reader<'a>,
    ctx: &ReaderContext<'a>,
) -> Option<Dict<'a>> {
    reader.skip_white_spaces();
    reader.forward_tag(b"xref")?;
    reader.skip_white_spaces();

    while let Some(header) = reader.read_without_context::<SubsectionHeader>() {
        reader.jump(reader.offset() + XREF_ENTRY_LEN * header.num_entries as usize);
    }

    reader.skip_white_spaces();
    reader.forward_tag(b"trailer")?;
    reader.skip_white_spaces();

    reader.read_with_context::<Dict>(ctx)
}

struct ObjectStream<'a> {
    data: &'a [u8],
    ctx: ReaderContext<'a>,
    offsets: Vec<(u32, usize)>,
}

impl<'a> ObjectStream<'a> {
    fn new(inner: Stream<'a>, data: &'a [u8], ctx: &ReaderContext<'a>) -> Option<Self> {
        let num_objects = inner.dict().get::<usize>(N)?;
        let first_offset = inner.dict().get::<usize>(FIRST)?;

        let mut r = Reader::new(data);

        let mut offsets = vec![];

        for _ in 0..num_objects {
            r.skip_white_spaces_and_comments();
            // Skip object number
            let obj_num = r.read_without_context::<u32>()?;
            r.skip_white_spaces_and_comments();
            let relative_offset = r.read_without_context::<usize>()?;
            offsets.push((obj_num, first_offset + relative_offset));
        }

        Some(Self {
            data,
            ctx: ctx.clone(),
            offsets,
        })
    }

    fn get<T>(&self, index: u32) -> Option<T>
    where
        T: ObjectLike<'a>,
    {
        let offset = self.offsets.get(index as usize)?.1;
        let mut r = Reader::new(self.data);
        r.jump(offset);
        r.skip_white_spaces_and_comments();

        r.read_with_context::<T>(&self.ctx)
    }
}
