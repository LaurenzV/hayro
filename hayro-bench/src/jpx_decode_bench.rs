use hayro_jpeg2000::{DecodeSettings, DecoderContext, Image};
use hayro_syntax::{Filter, Pdf};
use std::borrow::Cow;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const DEFAULT_ITERATIONS: usize = 10;

struct ExtractedJpx {
    name: String,
    data: &'static [u8],
    image: &'static Image<'static>,
    width: u32,
    height: u32,
    channels: usize,
    output_len: usize,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse()?;
    let images = load_jpx_images(&args.pdf_path)?;

    if images.is_empty() {
        return Err(format!(
            "no JPXDecode streams found in {}",
            args.pdf_path.display()
        ));
    }

    println!(
        "found {} JPX images in {}",
        images.len(),
        args.pdf_path.display()
    );

    for image in &images {
        println!(
            "{}: {} bytes, {}x{}, {} channels, {} output bytes",
            image.name,
            image.data.len(),
            image.width,
            image.height,
            image.channels,
            image.output_len
        );
    }

    let mut outputs = images
        .iter()
        .map(|image| vec![0; image.output_len])
        .collect::<Vec<_>>();
    let mut contexts = images
        .iter()
        .map(|_| DecoderContext::default())
        .collect::<Vec<_>>();

    decode_round(&images, &mut outputs, &mut contexts)?;

    let mut per_image = vec![Duration::ZERO; images.len()];
    let total_start = Instant::now();

    for _ in 0..args.iterations {
        for (idx, image) in images.iter().enumerate() {
            let start = Instant::now();
            decode_one(image, &mut outputs[idx], &mut contexts[idx])?;
            per_image[idx] += start.elapsed();
        }
    }

    let total = total_start.elapsed();
    println!();
    println!("iterations: {}", args.iterations);
    println!(
        "total: {:.3} ms ({:.3} ms/iteration)",
        total.as_secs_f64() * 1000.0,
        (total / args.iterations as u32).as_secs_f64() * 1000.0
    );

    for (image, elapsed) in images.iter().zip(per_image) {
        println!(
            "{}: {:.3} ms/decode",
            image.name,
            (elapsed / args.iterations as u32).as_secs_f64() * 1000.0
        );
    }

    Ok(())
}

fn load_jpx_images(pdf_path: &Path) -> Result<Vec<ExtractedJpx>, String> {
    let pdf_data = fs::read(pdf_path)
        .map_err(|err| format!("failed to read {}: {err}", pdf_path.display()))?;
    let pdf = Pdf::new(pdf_data).map_err(|err| format!("failed to parse PDF: {err:?}"))?;
    let mut images = Vec::new();

    for object in pdf.objects() {
        let Some(stream) = object.into_stream() else {
            continue;
        };

        if !stream
            .filters()
            .iter()
            .any(|filter| *filter == Filter::JpxDecode)
        {
            continue;
        }

        let object_id = stream.obj_id();
        let raw_data = stream.raw_data();
        let owned_data = match raw_data {
            Cow::Borrowed(data) => data.to_vec(),
            Cow::Owned(data) => data,
        };

        let name = format!(
            "{:03}_{}_{}.jp2",
            images.len(),
            object_id.obj_number,
            object_id.gen_number
        );
        let leaked_data = Box::leak(owned_data.into_boxed_slice());

        let settings = DecodeSettings::default();
        let image = Image::new(leaked_data, &settings).map_err(|err| {
            format!(
                "failed to parse JPX stream {}_{}: {err:?}",
                object_id.obj_number, object_id.gen_number
            )
        })?;
        let image = Box::leak(Box::new(image));
        let channels = image.color_space().num_channels() as usize + usize::from(image.has_alpha());
        let output_len = image.width() as usize * image.height() as usize * channels;

        images.push(ExtractedJpx {
            name,
            data: leaked_data,
            image,
            width: image.width(),
            height: image.height(),
            channels,
            output_len,
        });
    }

    Ok(images)
}

fn decode_round(
    images: &[ExtractedJpx],
    outputs: &mut [Vec<u8>],
    contexts: &mut [DecoderContext<'static>],
) -> Result<(), String> {
    for (idx, image) in images.iter().enumerate() {
        decode_one(image, &mut outputs[idx], &mut contexts[idx])?;
    }

    Ok(())
}

fn decode_one(
    extracted: &ExtractedJpx,
    output: &mut [u8],
    context: &mut DecoderContext<'static>,
) -> Result<(), String> {
    extracted
        .image
        .decode_into(output, context)
        .map_err(|err| format!("failed to decode {}: {err:?}", extracted.name))
}

struct Args {
    pdf_path: PathBuf,
    iterations: usize,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut pdf_path = None;
        let mut iterations = DEFAULT_ITERATIONS;
        let mut args = env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--pdf" => {
                    pdf_path = Some(
                        args.next()
                            .map(PathBuf::from)
                            .ok_or("--pdf needs a path".to_string())?,
                    );
                }
                "--iterations" => {
                    let value = args
                        .next()
                        .ok_or("--iterations needs a value".to_string())?;
                    iterations = value
                        .parse()
                        .map_err(|_| format!("invalid iteration count: {value}"))?;
                }
                "--help" | "-h" => {
                    return Err(usage());
                }
                _ if arg.starts_with('-') => return Err(format!("unknown argument: {arg}")),
                _ => {
                    if pdf_path.is_some() {
                        return Err(format!("unexpected positional argument: {arg}"));
                    }

                    pdf_path = Some(PathBuf::from(arg));
                }
            }
        }

        if iterations == 0 {
            return Err("iteration count must be greater than zero".to_string());
        }

        let pdf_path = pdf_path.ok_or_else(usage)?;

        Ok(Self {
            pdf_path,
            iterations,
        })
    }
}

fn usage() -> String {
    format!(
        "usage: cargo run -p hayro-bench --release --bin jpx_decode_bench -- <pdf> [--iterations {DEFAULT_ITERATIONS}]\n\
         also accepted: --pdf <pdf>"
    )
}
