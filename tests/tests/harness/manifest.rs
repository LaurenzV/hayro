use crate::WORKSPACE_PATH;
use serde_json::{Map, Value};
use std::ops::RangeInclusive;
use std::sync::LazyLock;

const MANIFESTS: [(&str, bool); 5] = [
    ("custom", false),
    ("pdfjs", true),
    ("pdfbox", true),
    ("pdfium", true),
    ("corpus", true),
];

static ENTRIES: LazyLock<Vec<Entry>> = LazyLock::new(|| {
    let mut entries = vec![];

    for (kind, downloaded_by_default) in MANIFESTS {
        let path = WORKSPACE_PATH.join(format!("manifest_{kind}.json"));
        let manifest = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("couldn't read {}: {err}", path.display()));
        let values: Vec<Value> = serde_json::from_str(&manifest)
            .unwrap_or_else(|err| panic!("couldn't parse {}: {err}", path.display()));

        entries.extend(
            values
                .iter()
                .map(|value| Entry::parse(value, kind, downloaded_by_default)),
        );
    }

    entries
});

pub(crate) fn entries() -> &'static [Entry] {
    &ENTRIES
}

pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) file: String,
    pub(crate) range: Option<RangeInclusive<usize>>,
    pub(crate) password: String,
    pub(crate) ignore: bool,
    pub(crate) svg: bool,
}

impl Entry {
    fn parse(value: &Value, kind: &str, downloaded_by_default: bool) -> Self {
        let empty = Map::new();
        let (id, fields) = match value {
            Value::String(id) => (id.as_str(), &empty),
            Value::Object(fields) => (fields["id"].as_str().unwrap(), fields),
            _ => panic!("invalid manifest entry in {kind}: {value}"),
        };

        let stub = id.replace(['-', '.'], "_");
        let name = if kind == "custom" {
            stub
        } else {
            format!("{kind}_{stub}")
        };

        let downloaded = fields
            .get("link")
            .and_then(Value::as_bool)
            .unwrap_or(downloaded_by_default);
        let file = if downloaded {
            format!("downloads/{kind}/{id}.pdf")
        } else {
            let file = fields
                .get("file")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if kind == "custom" {
                file.to_string()
            } else {
                format!("pdfs/{kind}/{}", file.trim_start_matches("pdfs/"))
            }
        };

        Self {
            name,
            file,
            range: page_range(fields),
            password: fields
                .get("password")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            ignore: fields
                .get("ignore")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            svg: fields.get("svg").and_then(Value::as_bool).unwrap_or(false),
        }
    }
}

fn page_range(fields: &Map<String, Value>) -> Option<RangeInclusive<usize>> {
    let page = |key: &str| {
        fields
            .get(key)
            .and_then(Value::as_u64)
            .map(|page| page as usize)
    };

    match (page("first_page"), page("last_page")) {
        (Some(first), Some(last)) => Some(first..=last),
        (Some(first), None) => Some(first..=usize::MAX),
        (None, Some(last)) => Some(0..=last),
        (None, None) => None,
    }
}
