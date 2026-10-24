use crate::harness::assets::FALLBACK_FONTS;
use crate::harness::manifest::entries;
use crate::harness::runner::{Outcome, Test};
use crate::harness::snapshot::{Suite, check};
use crate::{interpreter_settings, render_svg, svg_render_settings, try_load_pdf};

pub(crate) fn tests() -> Vec<Test> {
    entries()
        .iter()
        .filter(|entry| entry.svg && !entry.ignore)
        .map(|entry| {
            let mut files = vec![entry.file.clone()];
            files.extend(FALLBACK_FONTS.map(String::from));

            Test::new(format!("svg::{}", entry.name), files, move || {
                let pdf = match try_load_pdf(&entry.file, &entry.password) {
                    Ok(pdf) => pdf,
                    Err(err) => return Outcome::Failed(err),
                };
                let pages = render_svg(
                    &pdf,
                    &entry.name,
                    interpreter_settings(),
                    svg_render_settings(),
                    entry.range.clone(),
                );
                check(Suite::Svg, &entry.name, pages)
            })
        })
        .collect()
}
