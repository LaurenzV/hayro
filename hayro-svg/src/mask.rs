use crate::{Id, SvgRenderer, hash128};
use hayro_interpret::color::AlphaColor;
use hayro_interpret::{
    BlendMode, CacheKey, DrawMode, DrawProps, FillRule, MaskType, Paint, SoftMask, TransferFunction,
};
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
                    });
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
            if let MaskKind::SoftMask(mask) = mask
                && let Some(transfer_function) = mask.transfer_function()
            {
                self.write_transfer_function_filter(
                    &format!("f{id}"),
                    mask.mask_type(),
                    transfer_function,
                );
            }

            self.xml.start_element("mask");
            self.xml.write_attribute("id", &id);
            self.xml.write_attribute("maskUnits", "userSpaceOnUse");

            match mask {
                MaskKind::SoftMask(mask) => {
                    let filter_id = mask.transfer_function().map(|_| format!("f{id}"));

                    if mask.mask_type() != MaskType::Luminosity || filter_id.is_some() {
                        self.xml.write_attribute("mask-type", "alpha");
                    }

                    if let Some(filter_id) = &filter_id {
                        self.xml.start_element("g");
                        self.xml
                            .write_attribute("filter", &format!("url(#{filter_id})"));
                    }

                    let bg_color = mask.background_color();
                    let use_bg = bg_color.to_rgba().to_rgba8() != AlphaColor::BLACK.to_rgba8();

                    if use_bg {
                        let paint = Paint::Color(bg_color);
                        self.draw_path(
                            &Rect::new(
                                0.0,
                                0.0,
                                self.dimensions.0 as f64,
                                self.dimensions.1 as f64,
                            )
                            .to_path(0.1),
                            DrawProps {
                                transform: Affine::IDENTITY,
                                paint,
                                soft_mask: None,
                                blend_mode: BlendMode::Normal,
                            },
                            &DrawMode::Fill(FillRule::NonZero),
                        );
                        self.xml.start_element("g");
                        self.xml.write_attribute("style", "isolation:isolate");
                    }

                    mask.interpret(self);

                    if use_bg {
                        self.xml.end_element();
                    }

                    if filter_id.is_some() {
                        self.xml.end_element();
                    }
                }
                MaskKind::Image(i) => self.write_image(&i.image, i.interpolate, None, i.transform),
            }

            self.xml.end_element();
        }

        self.xml.end_element();
    }

    fn write_transfer_function_filter(
        &mut self,
        id: &str,
        mask_type: MaskType,
        transfer_function: &TransferFunction,
    ) {
        let table_values = sampled_transfer_function(transfer_function);

        self.xml.start_element("filter");
        self.xml.write_attribute("id", id);
        self.xml.write_attribute("filterUnits", "userSpaceOnUse");
        self.xml.write_attribute("x", "0");
        self.xml.write_attribute("y", "0");
        self.xml.write_attribute("width", &self.dimensions.0);
        self.xml.write_attribute("height", &self.dimensions.1);

        if mask_type == MaskType::Luminosity {
            self.xml.start_element("feColorMatrix");
            self.xml.write_attribute("type", "luminanceToAlpha");
            self.xml.end_element();
        }

        self.xml.start_element("feComponentTransfer");
        self.xml.start_element("feFuncA");
        self.xml.write_attribute("type", "table");
        self.xml.write_attribute("tableValues", &table_values);
        self.xml.end_element();
        self.xml.end_element();
        self.xml.end_element();
    }
}

fn sampled_transfer_function(transfer_function: &TransferFunction) -> String {
    (0..=255)
        .map(|i| transfer_function.apply(i as f32 / 255.0).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
