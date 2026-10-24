use crate::harness::assets::FALLBACK_FONTS;
use crate::harness::runner::{Outcome, Test};
use crate::harness::snapshot::{Suite, check};
use crate::{WORKSPACE_PATH, interpreter_settings, render_pdf, try_load_pdf};
use serde_json::Value;
use std::ops::RangeInclusive;

const MANIFESTS: [(&str, bool); 5] = [
    ("custom", false),
    ("pdfjs", true),
    ("pdfbox", true),
    ("pdfium", true),
    ("corpus", true),
];

/// Returns the raster tests of all manifests, as well as the names of the ignored ones.
pub(crate) fn tests() -> (Vec<Test>, Vec<String>) {
    let mut tests = vec![];
    let mut ignored = vec![];

    for (kind, downloaded_by_default) in MANIFESTS {
        let path = WORKSPACE_PATH.join(format!("manifest_{kind}.json"));
        let manifest = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("couldn't read {}: {err}", path.display()));
        let entries: Vec<Value> = serde_json::from_str(&manifest)
            .unwrap_or_else(|err| panic!("couldn't parse {}: {err}", path.display()));

        for entry in &entries {
            let entry = Entry::parse(entry, kind, downloaded_by_default);
            if entry.ignore {
                ignored.push(format!("raster::{}", entry.name));
            } else {
                tests.push(entry.into_test());
            }
        }
    }

    (tests, ignored)
}

struct Entry {
    name: String,
    file: String,
    range: Option<RangeInclusive<usize>>,
    password: String,
    ignore: bool,
}

impl Entry {
    fn parse(entry: &Value, kind: &str, downloaded_by_default: bool) -> Self {
        let (id, fields) = match entry {
            Value::String(id) => (id.as_str(), None),
            Value::Object(fields) => (fields["id"].as_str().unwrap(), Some(fields)),
            _ => panic!("invalid manifest entry in {kind}: {entry}"),
        };
        let field = |key: &str| fields.and_then(|fields| fields.get(key));
        let page = |key: &str| field(key).and_then(Value::as_u64).map(|page| page as usize);

        let stub = id.replace(['-', '.'], "_");
        let name = if kind == "custom" {
            stub
        } else {
            format!("{kind}_{stub}")
        };

        let downloaded = field("link")
            .and_then(Value::as_bool)
            .unwrap_or(downloaded_by_default);
        let file = if downloaded {
            format!("downloads/{kind}/{id}.pdf")
        } else {
            let file = field("file").and_then(Value::as_str).unwrap_or_default();
            if kind == "custom" {
                file.to_string()
            } else {
                format!("pdfs/{kind}/{}", file.trim_start_matches("pdfs/"))
            }
        };

        let range = match (page("first_page"), page("last_page")) {
            (Some(first), Some(last)) => Some(first..=last),
            (Some(first), None) => Some(first..=usize::MAX),
            (None, Some(last)) => Some(0..=last),
            (None, None) => None,
        };

        Self {
            name,
            file,
            range,
            password: field("password")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            ignore: field("ignore").and_then(Value::as_bool).unwrap_or(false),
        }
    }

    fn into_test(self) -> Test {
        let mut files = vec![self.file.clone()];
        files.extend(FALLBACK_FONTS.map(String::from));

        Test::new(format!("raster::{}", self.name), files, move || {
            let pdf = match try_load_pdf(&self.file, &self.password) {
                Ok(pdf) => pdf,
                Err(err) => return Outcome::Failed(err),
            };
            let pages = render_pdf(&pdf, &self.name, interpreter_settings(), self.range.clone());
            check(Suite::Raster, &self.name, pages)
        })
    }
}
