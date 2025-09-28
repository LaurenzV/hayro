use crate::object::{ObjectIdentifier, ObjectLike};
use crate::reader::{Readable, Reader, ReaderContext, Skippable};
use log::warn;

#[derive(Debug, Clone)]
pub(crate) struct IndirectObject<T> {
    id: ObjectIdentifier,
    inner: T,
}

impl<T> IndirectObject<T> {
    pub(crate) fn get(self) -> T {
        self.inner
    }

    pub(crate) fn id(&self) -> &ObjectIdentifier {
        &self.id
    }
}

impl<'a, T> Readable<'a> for IndirectObject<T>
where
    T: ObjectLike<'a>,
{
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let mut ctx = ctx.clone();
        let id = r.read_without_context::<ObjectIdentifier>()?;

        if ctx.parent_chain.contains(&id) {
            warn!("cycle detected in indirect object: {id:?}");

            return None;
        }

        ctx.obj_number = Some(id);
        ctx.parent_chain.push(id);
        r.skip_white_spaces_and_comments();
        let inner = r.read_with_context::<T>(&ctx)?;
        r.skip_white_spaces_and_comments();
        // We are lenient and don't require it.
        r.forward_tag(b"endobj");

        Some(Self { id, inner })
    }
}

impl<T> Skippable for IndirectObject<T>
where
    T: Skippable,
{
    fn skip(r: &mut Reader<'_>, _: bool) -> Option<()> {
        r.skip_in_content_stream::<ObjectIdentifier>()?;
        r.skip_white_spaces_and_comments();
        r.skip_not_in_content_stream::<T>()?;
        r.skip_white_spaces_and_comments();
        // We are lenient and don't require it.
        r.forward_tag(b"endobj");

        Some(())
    }
}
