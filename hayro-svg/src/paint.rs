use crate::Id;
use crate::render::SvgRenderer;
use hayro_interpret::pattern::{Pattern, ShadingPattern, TilingPattern};
use hayro_interpret::{CacheKey, Paint};
use kurbo::{Affine, BezPath, Rect, Shape};

#[derive(Clone)]
pub(crate) struct CachedTilingPattern<'a> {
    pub(crate) transform: Affine,
    pub(crate) tiling_pattern: TilingPattern<'a>,
}

pub(crate) struct CachedShadingPattern {
    pub(crate) transform: Affine,
    pub(crate) shading: Id,
    pub(crate) bbox: Rect,
}

pub(crate) struct CachedShading {
    pub(crate) pattern: ShadingPattern,
    pub(crate) bbox: Rect,
}

impl<'a> SvgRenderer<'a> {
    pub(crate) fn write_paint(
        &mut self,
        paint: &Paint<'a>,
        path: &BezPath,
        path_transform: Affine,
        is_stroke: bool,
    ) {
        let (paint_str, alpha) = match &paint {
            Paint::Color(c) => {
                let rgba8 = c.to_rgba().to_rgba8();
                let color = format!(
                    "#{}",
                    &rgba8[0..3]
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                );
                let alpha = rgba8[3] as f32 / 255.0;

                (color, alpha)
            }
            Paint::Pattern(p) => match p.as_ref() {
                Pattern::Shading(s) => {
                    let bbox = (path_transform * path).bounding_box();
                    let shading_id = self.shadings.insert_with(s.cache_key(), || CachedShading {
                        pattern: s.clone(),
                        bbox,
                    });

                    let inverse_transform = path_transform.inverse();
                    let pattern_id = self.shading_patterns.insert_with(
                        (s.clone(), inverse_transform).cache_key(),
                        || CachedShadingPattern {
                            transform: inverse_transform,
                            bbox,
                            shading: shading_id,
                        },
                    );

                    (format!("url(#{pattern_id})"), 1.0)
                }
                Pattern::Tiling(t) => {
                    let inverse_transform = path_transform.inverse();
                    let pattern = *t.clone();

                    let pattern_id = self.tiling_patterns.insert_with(
                        (pattern.clone(), inverse_transform).cache_key(),
                        || CachedTilingPattern {
                            transform: inverse_transform,
                            tiling_pattern: pattern,
                        },
                    );

                    (format!("url(#{pattern_id})"), 1.0)
                }
            },
        };

        if is_stroke {
            self.xml.write_attribute("fill", "none");
            self.xml.write_attribute("stroke", &paint_str);
            if alpha != 1.0 {
                self.xml.write_attribute("stroke-opacity", &alpha);
            }
        } else {
            self.xml.write_attribute("fill", &paint_str);

            if alpha != 1.0 {
                self.xml.write_attribute("fill-opacity", &alpha);
            }
        }
    }
}
