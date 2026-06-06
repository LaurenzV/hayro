#![allow(missing_docs)]

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use hayro_jpeg2000::{DecodeSettings, DecoderContext, Image};
use serde::Deserialize;

const INPUT_MANIFESTS: &[(&str, &str)] = &[
    ("openjpeg", "manifest_openjpeg.json"),
    ("serenity", "manifest_serenity.json"),
];

#[derive(Deserialize)]
#[serde(untagged)]
enum ManifestItem {
    Simple(String),
    Detailed(ManifestEntry),
}

#[derive(Deserialize)]
struct ManifestEntry {
    id: String,
    #[serde(default, alias = "file")]
    path: String,
    #[serde(default = "default_render")]
    render: bool,
    #[serde(default)]
    strict: Option<bool>,
    #[serde(default)]
    resolve_palette_indices: Option<bool>,
    #[serde(default)]
    target_resolution: Option<(u32, u32)>,
}

struct BenchAsset {
    name: String,
    path: PathBuf,
    decode_settings: DecodeSettings,
}

impl ManifestItem {
    fn into_asset(self, namespace: &str) -> Option<BenchAsset> {
        let default_settings = DecodeSettings::default();

        match self {
            Self::Simple(id) => Some(BenchAsset {
                name: format!("{namespace}/{id}"),
                path: Path::new(namespace).join(&id),
                decode_settings: default_settings,
            }),
            Self::Detailed(entry) => {
                if !entry.render {
                    return None;
                }

                Some(BenchAsset {
                    name: format!("{namespace}/{}", entry.id),
                    path: Path::new(namespace).join(entry.path),
                    decode_settings: DecodeSettings {
                        resolve_palette_indices: entry
                            .resolve_palette_indices
                            .unwrap_or(default_settings.resolve_palette_indices),
                        strict: entry.strict.unwrap_or(default_settings.strict),
                        target_resolution: entry
                            .target_resolution
                            .or(default_settings.target_resolution),
                    },
                })
            }
        }
    }
}

fn default_render() -> bool {
    true
}

fn collect_assets() -> Vec<BenchAsset> {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut assets = Vec::new();

    for (namespace, manifest_name) in INPUT_MANIFESTS {
        let manifest_path = crate_dir.join(manifest_name);
        let manifest = fs::read_to_string(&manifest_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", manifest_path.display()));
        let entries = serde_json::from_str::<Vec<ManifestItem>>(&manifest)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", manifest_path.display()));

        for entry in entries {
            if let Some(mut asset) = entry.into_asset(namespace) {
                asset.path = crate_dir.join("test-inputs").join(asset.path);

                if asset.path.exists() {
                    assets.push(asset);
                }
            }
        }
    }

    assets.sort_by(|a, b| a.name.cmp(&b.name));
    let mut seen_names = HashSet::new();
    assets.retain(|asset| seen_names.insert(asset.name.clone()));
    assets
}

fn output_len(image: &Image<'_>) -> usize {
    let channels = image.color_space().num_channels() as usize + usize::from(image.has_alpha());
    image.width() as usize * image.height() as usize * channels
}

fn bench_decode(c: &mut Criterion) {
    let assets = collect_assets();
    assert!(
        !assets.is_empty(),
        "missing test-inputs; run `python sync.py` in hayro-jpeg2000"
    );

    let mut group = c.benchmark_group("decode");

    for asset in assets {
        let data = fs::read(&asset.path).unwrap();
        let Ok(image) = Image::new(&data, &asset.decode_settings) else {
            continue;
        };
        let mut output = vec![0; output_len(&image)];

        group.bench_function(&asset.name, |b| {
            let mut ctx = DecoderContext::default();
            b.iter(|| {
                image
                    .decode_into(black_box(output.as_mut_slice()), &mut ctx)
                    .unwrap();
                black_box(&output);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_decode);
criterion_main!(benches);
