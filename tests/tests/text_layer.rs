use crate::harness::assets::FALLBACK_FONTS;
use crate::harness::manifest::entries;
use crate::harness::runner::{Outcome, Test};
use crate::{interpreter_settings, load_pdf, svg_render_settings, try_load_pdf};
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Rect};
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, SoftMask, interpret_page,
};
use hayro_svg::{RenderCache, SvgRenderSettings};
use hayro_syntax::page::Page;
use resvg::usvg::roxmltree::{Document, Node};
use std::fmt::Write;

pub(crate) fn tests() -> Vec<Test> {
    let units: [(&str, fn()); 2] = [
        ("text_layer::layout", layout),
        ("text_layer::invisible", invisible),
    ];

    entries()
        .iter()
        .filter(|entry| entry.svg && !entry.ignore)
        .map(|entry| {
            let mut files = vec![entry.file.clone()];
            files.extend(FALLBACK_FONTS.map(String::from));

            Test::new(format!("text_layer::{}", entry.name), files, move || {
                let pdf = match try_load_pdf(&entry.file, &entry.password) {
                    Ok(pdf) => pdf,
                    Err(err) => return Outcome::Failed(err),
                };

                for (idx, page) in pdf.pages().iter().enumerate() {
                    if entry
                        .range
                        .clone()
                        .is_some_and(|range| !range.contains(&idx))
                    {
                        continue;
                    }

                    if let Err(err) = check(page) {
                        return Outcome::Failed(format!("page {idx}: {err}"));
                    }
                }

                Outcome::Passed
            })
        })
        .chain(units.map(|(name, run)| Test::unit(name, run)))
        .collect()
}

/// Check that the SVG of a page with a text layer is well-formed, that it only adds the
/// layer to the SVG without one, and that the layer holds the text of the page's glyphs.
fn check(page: &Page<'_>) -> Result<(), String> {
    let plain = convert(page, false);
    let layered = convert(page, true);
    let plain = Document::parse(&plain).map_err(|err| format!("invalid SVG: {err}"))?;
    let layered = Document::parse(&layered).map_err(|err| format!("invalid SVG: {err}"))?;

    if content(&layered) != content(&plain) {
        return Err("the text layer changes the rest of the SVG".to_string());
    }

    let mut text = String::new();
    for element in texts(&layered) {
        let chars = element.text().unwrap_or_default();
        let positions = element.attribute("x").map_or(0, |x| x.split(' ').count());
        if positions != chars.chars().count() {
            return Err(format!("{positions} positions for {chars:?}"));
        }
        text.push_str(chars);
    }

    let expected = page_text(page);
    if text != expected {
        return Err(format!(
            "the text layer holds {text:?} instead of {expected:?}"
        ));
    }

    Ok(())
}

fn convert(page: &Page<'_>, text_layer: bool) -> String {
    let settings = SvgRenderSettings {
        text_layer,
        ..svg_render_settings()
    };

    hayro_svg::convert(
        page,
        &RenderCache::new(),
        &interpreter_settings(),
        &settings,
    )
}

fn texts<'a, 'input>(document: &'a Document<'input>) -> Vec<Node<'a, 'input>> {
    document
        .root_element()
        .children()
        .filter(|node| node.attribute("id") == Some("text-layer"))
        .flat_map(|layer| layer.children().filter(Node::is_element))
        .collect()
}

/// The elements and the text of an SVG, except for its text layer.
fn content(document: &Document<'_>) -> Vec<String> {
    let mut content = vec![];

    for node in document.descendants() {
        if node
            .ancestors()
            .any(|node| node.attribute("id") == Some("text-layer"))
        {
            continue;
        }

        if node.is_element() {
            let mut element = format!("<{}", node.tag_name().name());
            for attribute in node.attributes() {
                write!(element, " {}={:?}", attribute.name(), attribute.value()).unwrap();
            }
            content.push(element);
        } else if let Some(text) = node.text().filter(|text| !text.trim().is_empty()) {
            content.push(text.to_string());
        }
    }

    content
}

/// The text of the glyphs of a page that have a Unicode mapping and aren't drawn at zero
/// size, with the characters that the text layer writes. A run that draws the same glyphs
/// at the same place as the previous one adds no text.
fn page_text(page: &Page<'_>) -> String {
    let cache = InterpreterCache::new();
    let (width, height) = page.render_dimensions();
    let mut context = Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0.0, 0.0, width as f64, height as f64),
        &cache,
        page.xref(),
        interpreter_settings(),
    );
    let mut collector = TextCollector::default();
    interpret_page(page, &mut context, &mut collector);

    collector
        .text
        .chars()
        .filter_map(|c| match c {
            '\t' | '\n' | '\r' => Some(' '),
            '\0'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}' => None,
            c => Some(c),
        })
        .collect()
}

#[derive(Default)]
struct TextCollector {
    text: String,
    last_run: Vec<(Affine, u32)>,
}

impl<'a> Device<'a> for TextCollector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(
        &mut self,
        _: f32,
        _: Option<SoftMask<'a>>,
        _: BlendMode,
        _: bool,
        _: bool,
    ) {
    }
    fn draw_glyph_run(&mut self, glyph_run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        let run: Vec<_> = glyph_run
            .glyphs()
            .iter()
            .map(|glyph| (props.transform * glyph.transform(), glyph.char_code()))
            .collect();
        if run == self.last_run {
            return;
        }

        for (glyph, (transform, _)) in glyph_run.glyphs().iter().zip(&run) {
            if !transform.is_finite() || !transform.determinant().is_normal() {
                continue;
            }

            match glyph.as_unicode() {
                Some(BfString::Char(c)) => self.text.push(c),
                Some(BfString::String(s)) => self.text.push_str(&s),
                None => {}
            }
        }
        self.last_run = run;
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

struct Text {
    text: String,
    transform: String,
    x: String,
    y: Option<String>,
}

/// Check the first page of a PDF and return the text elements of its text layer, as well
/// as its SVG without the layer.
fn layer(path: &str) -> (Vec<Text>, String) {
    let pdf = load_pdf(path);
    let page = &pdf.pages()[0];
    check(page).unwrap();

    let svg = convert(page, true);
    let document = Document::parse(&svg).unwrap();
    let texts = texts(&document)
        .iter()
        .map(|element| Text {
            text: element.text().unwrap_or_default().to_string(),
            transform: element.attribute("transform").unwrap().to_string(),
            x: element.attribute("x").unwrap().to_string(),
            y: element.attribute("y").map(str::to_string),
        })
        .collect();

    (texts, convert(page, false))
}

fn layout() {
    let (texts, _) = layer("pdfs/text/text_layer.pdf");

    // The text of the Type 3 glyph's procedure, the tiling pattern and the soft mask
    // isn't recorded, and the filled and stroked text is recorded once.
    assert_eq!(
        texts.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
        ["Fish & <Chips>", "fi", "outlined", "up", "縦書", "a"]
    );
    // The two characters of the ligature split its advance.
    assert_eq!(texts[1].x, "0 250");
    assert_eq!(texts[1].y, None);
    // The rotated text keeps the rotation of its glyphs.
    assert!(texts[3].transform.starts_with("matrix(0 -"));
    // The vertical text runs down.
    assert_eq!(texts[4].x, "0 0");
    assert_eq!(texts[4].y.as_deref(), Some("0 1000"));
}

fn invisible() {
    let (texts, plain) = layer("pdfs/text/text_layer_invisible.pdf");

    assert_eq!(
        texts.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
        ["Scanned page", "with OCR text"]
    );
    // Without the layer, the text leaves no trace.
    assert!(!plain.contains("<use"));
}
