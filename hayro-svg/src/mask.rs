use crate::{Id, SvgRenderer};
use hayro_interpret::{CacheKey, MaskType, SoftMask};

#[derive(Clone)]
pub(crate) struct CachedMask<'a>(SoftMask<'a>);

impl<'a> SvgRenderer<'a> {
    pub(crate) fn get_mask_id(&mut self, mask: SoftMask<'a>) -> Id {
        let cache_key = mask.cache_key();

        if !self.masks.contains(cache_key) {
            self.with_dummy(|r| {
                mask.interpret(r);
            })
        }

        self.masks.insert_with(cache_key, || CachedMask(mask))
    }

    pub(crate) fn write_mask_defs(&mut self) {
        if self.masks.is_empty() {
            return;
        }

        self.xml.start_element("defs");
        self.xml.write_attribute("id", "mask");

        let masks = self.masks.clone();

        for (id, mask) in masks.iter() {
            self.xml.start_element("mask");
            self.xml.write_attribute("id", &id);
            self.xml.write_attribute("maskUnits", "userSpaceOnUse");

            if mask.0.mask_type() != MaskType::Luminosity {
                self.xml.write_attribute("mask-type", "alpha");
            }

            mask.0.interpret(self);

            self.xml.end_element();
        }

        self.xml.end_element();
    }
}
