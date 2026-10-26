use crate::WORKSPACE_PATH;
use crate::harness::runner::{Outcome, Test};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::render_pdf;
use hayro_jbig2::DecoderContext;
use hayro_jpeg2000::{DecodeSettings, Image};
use hayro_syntax::Pdf;
use hayro_syntax::metadata::Metadata;
use hayro_syntax::object::{DateTime, Dict, ObjectIdentifier};
use std::sync::Barrier;

fn load_pdf(file: &[u8]) {
    let pdf = Pdf::new(file.to_vec());

    if let Ok(pdf) = pdf {
        let _pixmaps = render_pdf(&pdf, 1.0, InterpreterSettings::default(), None);
    }
}

fn load_jbig2(file: &[u8]) {
    struct NullDecoder;
    impl hayro_jbig2::Decoder for NullDecoder {
        fn push_pixel(&mut self, _black: bool) {}
        fn push_pixel_chunk(&mut self, _black: bool, _chunk_count: u32) {}
        fn next_line(&mut self) {}
    }

    if let Ok(file) = hayro_jbig2::File::new(file, &hayro_jbig2::DecodeSettings::default()) {
        let mut ctx = DecoderContext::default();
        for page in file.pages() {
            let _ = page.decode_with(&mut NullDecoder, &mut ctx);
        }
    }
}

fn load_jpeg2000(file: &[u8]) {
    use image::ImageDecoder;

    let settings = DecodeSettings::default();
    if let Ok(image) = Image::new(file, &settings) {
        let mut buf = vec![0_u8; image.total_bytes() as usize];
        let _ = image.read_image(&mut buf);
    }
}

pub(crate) fn tests() -> Vec<Test> {
    let dir = WORKSPACE_PATH.join("pdfs/load");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("couldn't read {}: {err}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();

    let files = files.into_iter().filter_map(|path| {
        let loader: fn(&[u8]) = match path.extension()?.to_str()? {
            "pdf" => load_pdf,
            "jb2" => load_jbig2,
            "jp2" => load_jpeg2000,
            _ => return None,
        };
        let stem = path.file_stem()?.to_str()?.replace(['-', '.'], "_");
        let file = path
            .strip_prefix(&*WORKSPACE_PATH)
            .ok()?
            .to_str()?
            .to_string();

        Some(Test::new(
            format!("load::{stem}"),
            vec![file.clone()],
            move || {
                loader(&read(&file));
                Outcome::Passed
            },
        ))
    });

    let units: [(&str, fn()); 4] = [
        ("load::metadata_in_object_stream", metadata_in_object_stream),
        (
            "load::concurrent_object_stream_resolution",
            concurrent_object_stream_resolution,
        ),
        (
            "load::concurrent_xref_repair_does_not_panic",
            concurrent_xref_repair_does_not_panic,
        ),
        (
            "load::concurrent_xref_repair_retries_original_entries",
            concurrent_xref_repair_retries_original_entries,
        ),
    ];

    files
        .chain(units.map(|(name, run)| Test::unit(name, run)))
        .collect()
}

fn read(path: &str) -> Vec<u8> {
    std::fs::read(WORKSPACE_PATH.join(path)).unwrap()
}

fn metadata_in_object_stream() {
    // Normally, in an encrypted PDF file strings need to be encrypted when they are not
    // in a stream. Therefore, we need to ensure that no encryption is applied when the object
    // itself is in an object stream.
    let file = &read("pdfs/custom/metadata_in_object_stream_encrypted.pdf");
    let pdf = Pdf::new(file.to_vec()).unwrap();

    let expected = Metadata {
        creation_date: Some(DateTime {
            year: 2025,
            month: 10,
            day: 26,
            hour: 16,
            minute: 24,
            second: 17,
            utc_offset_hour: 1,
            utc_offset_minute: 0,
        }),
        modification_date: Some(DateTime {
            year: 2025,
            month: 10,
            day: 26,
            hour: 16,
            minute: 33,
            second: 30,
            utc_offset_hour: 1,
            utc_offset_minute: 0,
        }),
        title: Some(Box::from(b"Encrypted Metadata".as_slice())),
        author: Some(Box::from(b"Max Mustermann".as_slice())),
        subject: None,
        keywords: None,
        creator: Some(Box::from(b"Typst 0.14.0".as_slice())),
        producer: None,
    };

    assert_eq!(pdf.metadata(), &expected);
}

fn concurrent_object_stream_resolution() {
    let data = &read("pdfs/load/concurrent_object_streams.pdf");

    for _ in 0..32 {
        let pdf = Pdf::new(data.to_vec()).unwrap();
        let barrier = Barrier::new(8);

        std::thread::scope(|scope| {
            for thread in 0..8 {
                let pdf = &pdf;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();

                    // Objects 6..30 are split between two compressed object streams.
                    for offset in 0..24 {
                        let number = 6 + (thread * 12 + offset) % 24;
                        let value = pdf
                            .xref()
                            .get::<Dict<'_>>(ObjectIdentifier::new(number, 0))
                            .and_then(|dict| dict.get::<i32>(b"V"));
                        assert_eq!(value, Some(100 + number));
                    }
                });
            }
        });
    }
}

fn concurrent_xref_repair_does_not_panic() {
    let data = &read("pdfs/load/concurrent_xref_repair.pdf");

    for _ in 0..32 {
        let pdf = Pdf::new(data.to_vec()).unwrap();
        let barrier = Barrier::new(8);

        std::thread::scope(|scope| {
            for thread in 0..8 {
                let pdf = &pdf;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();

                    if thread % 2 == 0 {
                        let _ = pdf
                            .xref()
                            .get::<Dict<'_>>(ObjectIdentifier::new(3 + thread, 0));
                    } else {
                        // The catalog's offset is valid even before repair.
                        assert!(pdf.xref().get::<Dict<'_>>(pdf.xref().root_id()).is_some());
                    }
                });
            }
        });

        let value = pdf
            .xref()
            .get::<Dict<'_>>(ObjectIdentifier::new(3, 0))
            .and_then(|dict| dict.get::<i32>(b"V"));
        assert_eq!(value, Some(103));
    }
}

fn concurrent_xref_repair_retries_original_entries() {
    let data = &read("pdfs/load/concurrent_xref_repair.pdf");

    for _ in 0..32 {
        let pdf = Pdf::new(data.to_vec()).unwrap();
        let barrier = Barrier::new(8);

        std::thread::scope(|scope| {
            for thread in 0..8 {
                let pdf = &pdf;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();

                    // Objects 3..11 have incorrect xref offsets.
                    for offset in 0..8 {
                        let number = 3 + (thread + offset) % 8;
                        let value = pdf
                            .xref()
                            .get::<Dict<'_>>(ObjectIdentifier::new(number, 0))
                            .and_then(|dict| dict.get::<i32>(b"V"));
                        assert_eq!(value, Some(100 + number));
                    }
                });
            }
        });
    }
}
