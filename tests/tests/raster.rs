use crate::harness::assets::FALLBACK_FONTS;
use crate::harness::manifest::entries;
use crate::harness::runner::{Outcome, Test};
use crate::harness::snapshot::{Suite, check};
use crate::{interpreter_settings, render_pdf, try_load_pdf};

/// Returns the raster tests of all manifests, as well as the names of the ignored ones.
pub(crate) fn tests() -> (Vec<Test>, Vec<String>) {
    let mut tests = vec![];
    let mut ignored = vec![];

    for entry in entries() {
        let name = format!("raster::{}", entry.name);
        if entry.ignore {
            ignored.push(name);
            continue;
        }

        let mut files = vec![entry.file.clone()];
        files.extend(FALLBACK_FONTS.map(String::from));

        tests.push(Test::new(name, files, move || {
            let pdf = match try_load_pdf(&entry.file, &entry.password) {
                Ok(pdf) => pdf,
                Err(err) => return Outcome::Failed(err),
            };
            let pages = render_pdf(
                &pdf,
                &entry.name,
                interpreter_settings(),
                entry.range.clone(),
            );
            check(Suite::Raster, &entry.name, pages)
        }));
    }

    (tests, ignored)
}
