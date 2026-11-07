use crate::harness::runner::Test;
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Rect};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;

pub(crate) fn tests() -> Vec<Test> {
    let units: [(&str, fn()); 1] = [(
        "groups::isolated_and_knockout_flags",
        isolated_and_knockout_flags,
    )];

    units.map(|(name, run)| Test::unit(name, run)).into()
}

/// Records each pushed group and the flags reported for it.
#[derive(Default)]
struct GroupLog(Vec<String>);

impl<'a> Device<'a> for GroupLog {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {
        self.0.push("group".into());
    }
    fn set_group_flags(&mut self, isolated: bool, knockout: bool) {
        if let Some(last) = self.0.last_mut() {
            last.push_str(&format!(" I={isolated} K={knockout}"));
        }
    }
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes(),
    );
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn form(group: &str) -> Vec<u8> {
    let data = b"0 0 10 10 re f";
    let mut v = format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] {group} /Length {} >>\nstream\n",
        data.len()
    )
    .into_bytes();
    v.extend_from_slice(data);
    v.extend_from_slice(b"\nendstream");
    v
}

fn isolated_and_knockout_flags() {
    let content = b"/A Do /B Do /C Do";
    let mut contents = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    contents.extend_from_slice(content);
    contents.extend_from_slice(b"\nendstream");
    let file = pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /XObject << /A 5 0 R /B 6 0 R /C 7 0 R >> >> >>".to_vec(),
        contents,
        form("/Group << /S /Transparency /K true >>"),
        form("/Group << /S /Transparency /I true >>"),
        form(""),
    ]);
    let pdf = Pdf::new(file).unwrap();
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        &cache,
        pdf.xref(),
        crate::interpreter_settings(),
    );
    let mut log = GroupLog::default();
    interpret_page(&pdf.pages()[0], &mut context, &mut log);

    let forms: Vec<&String> = log.0.iter().filter(|g| g.contains("I=")).collect();
    assert_eq!(
        forms,
        ["group I=false K=true", "group I=true K=false"],
        "{:?}",
        log.0
    );
}
