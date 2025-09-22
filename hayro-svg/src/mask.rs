use crate::{Id, SvgRenderer, hash128};
use hayro_interpret::color::AlphaColor;
use hayro_interpret::{CacheKey, FillRule, MaskType, Paint, PathDrawMode, SoftMask};
use image::DynamicImage;
use kurbo::{Affine, Rect, Shape};
use std::sync::Arc;

pub(crate) struct ImageLuminanceMask {
    pub(crate) image: DynamicImage,
    pub(crate) transform: Affine,
    pub(crate) interpolate: bool,
}

#[derive(Clone)]
pub(crate) enum MaskKind<'a> {
    SoftMask(SoftMask<'a>),
    Image(Arc<ImageLuminanceMask>),
}

impl<'a> SvgRenderer<'a> {
    pub(crate) fn get_mask_id(&mut self, mask: MaskKind<'a>) -> Id {
        match mask {
            MaskKind::SoftMask(mask) => {
                let cache_key = mask.cache_key();

                if !self.masks.contains(cache_key) {
                    self.with_dummy(|r| {
                        mask.interpret(r);
                    })
                }

                self.masks
                    .insert_with(cache_key, || MaskKind::SoftMask(mask))
            }
            MaskKind::Image(mask) => {
                let cache_key = hash128(&(
                    mask.interpolate,
                    mask.transform.cache_key(),
                    mask.image.as_bytes(),
                ));

                self.masks.insert_with(cache_key, || MaskKind::Image(mask))
            }
        }
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

            match mask {
                MaskKind::SoftMask(mask) => {
                    if mask.mask_type() != MaskType::Luminosity {
                        self.xml.write_attribute("mask-type", "alpha");
                    }

                    let bg_color = mask.background_color();
                    let use_bg = bg_color.to_rgba().to_rgba8() != AlphaColor::BLACK.to_rgba8();

                    if use_bg {
                        self.draw_path(
                            &Rect::new(
                                0.0,
                                0.0,
                                self.dimensions.0 as f64,
                                self.dimensions.1 as f64,
                            )
                            .to_path(0.1),
                            Affine::IDENTITY,
                            &Paint::Color(bg_color),
                            &PathDrawMode::Fill(FillRule::NonZero),
                        );
                        self.xml.start_element("g");
                        self.xml.write_attribute("style", "isolation:isolate");
                    }

                    mask.interpret(self);

                    if use_bg {
                        self.xml.end_element();
                    }
                }
                MaskKind::Image(i) => self.write_image(&i.image, i.interpolate, None, i.transform),
            }

            self.xml.end_element();
        }

        self.xml.end_element();
    }
}
