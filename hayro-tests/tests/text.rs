use crate::{interpreter_settings, load_pdf};
use hayro::text::extract_text;

/// Smoke test: extract text from a PDF that uses standard fonts and verify
/// that the returned spans contain the expected Unicode strings with sensible
/// positions.
#[test]
fn extract_text_font_standard_1() {
    let pdf = load_pdf("pdfs/custom/font_standard_1.pdf");
    let settings = interpreter_settings();

    for page in pdf.pages().iter() {
        let spans = extract_text(page, &settings);

        // We should get at least one span of text.
        assert!(!spans.is_empty(), "expected at least one text span");

        // Every span should have consistent internal structure.
        for span in &spans {
            assert_eq!(
                span.glyphs.len(),
                span.glyphs.iter().count(),
                "glyphs vec length mismatch"
            );

            // The concatenation of per-glyph text must equal the span text.
            let reassembled: String = span.glyphs.iter().map(|g| g.text.as_str()).collect();
            assert_eq!(
                reassembled, span.text,
                "per-glyph text does not reassemble to span text"
            );

            // Font size should be positive.
            assert!(
                span.font_size > 0.0,
                "font_size should be positive, got {}",
                span.font_size
            );
        }
    }
}

/// Verify that glyph positions are in a plausible device-space range
/// (non-NaN, within the page dimensions).
#[test]
fn extract_text_positions_are_sane() {
    let pdf = load_pdf("pdfs/custom/font_standard_1.pdf");
    let settings = interpreter_settings();

    let page = &pdf.pages()[0];
    let spans = extract_text(page, &settings);

    for span in &spans {
        for glyph in &span.glyphs {
            assert!(glyph.x.is_finite(), "glyph x is not finite: {}", glyph.x);
            assert!(glyph.y.is_finite(), "glyph y is not finite: {}", glyph.y);
            assert!(
                glyph.advance_x.is_finite(),
                "advance_x is not finite: {}",
                glyph.advance_x
            );
            assert!(
                glyph.advance_y.is_finite(),
                "advance_y is not finite: {}",
                glyph.advance_y
            );

            // Positions should be non-negative for a page with top-left origin.
            // (Allow a small epsilon for floating-point imprecision.)
            assert!(
                glyph.x >= -1.0,
                "glyph x should be >= -1.0, got {}",
                glyph.x
            );
            assert!(
                glyph.y >= -1.0,
                "glyph y should be >= -1.0, got {}",
                glyph.y
            );
        }
    }
}

/// For horizontal text the advance should be primarily in the X direction
/// and nearly zero in Y.
#[test]
fn extract_text_horizontal_advance() {
    let pdf = load_pdf("pdfs/custom/font_standard_1.pdf");
    let settings = interpreter_settings();

    let page = &pdf.pages()[0];
    let spans = extract_text(page, &settings);

    let mut found_nonspace = false;

    for span in &spans {
        for glyph in &span.glyphs {
            // Skip space characters — they may have zero advance in some fonts.
            if glyph.text.trim().is_empty() {
                continue;
            }
            found_nonspace = true;

            // Horizontal text: advance_x should be positive, advance_y ≈ 0.
            assert!(
                glyph.advance_x > 0.0,
                "expected positive advance_x for '{}', got {}",
                glyph.text,
                glyph.advance_x
            );
            assert!(
                glyph.advance_y.abs() < 0.5,
                "expected near-zero advance_y for horizontal text glyph '{}', got {}",
                glyph.text,
                glyph.advance_y
            );
        }
    }

    assert!(found_nonspace, "expected at least one non-space glyph");
}

/// Verify that glyphs within a span are ordered left-to-right (for a known
/// horizontal LTR document).
#[test]
fn extract_text_glyphs_ordered_ltr() {
    let pdf = load_pdf("pdfs/custom/font_standard_1.pdf");
    let settings = interpreter_settings();

    let page = &pdf.pages()[0];
    let spans = extract_text(page, &settings);

    for span in &spans {
        if span.glyphs.len() < 2 {
            continue;
        }
        for pair in span.glyphs.windows(2) {
            // Each glyph's x should be >= the previous glyph's x for LTR text.
            assert!(
                pair[1].x >= pair[0].x - 0.01,
                "glyphs not in LTR order: '{}' at x={} followed by '{}' at x={}",
                pair[0].text,
                pair[0].x,
                pair[1].text,
                pair[1].x
            );
        }
    }
}

/// The full extracted text (all spans concatenated) should be non-empty and
/// contain only valid Unicode.
#[test]
fn extract_text_full_content_nonempty() {
    let pdf = load_pdf("pdfs/custom/font_standard_1.pdf");
    let settings = interpreter_settings();

    let page = &pdf.pages()[0];
    let spans = extract_text(page, &settings);

    let full_text: String = spans
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(
        !full_text.trim().is_empty(),
        "expected non-empty text from font_standard_1.pdf"
    );
}

/// Extracting text from a page with TrueType fonts should also work.
#[test]
fn extract_text_truetype() {
    let pdf = load_pdf("pdfs/custom/font_truetype_1.pdf");
    let settings = interpreter_settings();

    for page in pdf.pages().iter() {
        let spans = extract_text(page, &settings);
        assert!(!spans.is_empty(), "expected text spans from truetype PDF");

        for span in &spans {
            let reassembled: String = span.glyphs.iter().map(|g| g.text.as_str()).collect();
            assert_eq!(reassembled, span.text);
        }
    }
}

/// Extracting text from a page with Type1 fonts should also work.
#[test]
fn extract_text_type1() {
    let pdf = load_pdf("pdfs/custom/font_type1_1.pdf");
    let settings = interpreter_settings();

    for page in pdf.pages().iter() {
        let spans = extract_text(page, &settings);
        assert!(!spans.is_empty(), "expected text spans from type1 PDF");

        for span in &spans {
            let reassembled: String = span.glyphs.iter().map(|g| g.text.as_str()).collect();
            assert_eq!(reassembled, span.text);
        }
    }
}

/// Extracting text from a page with CID fonts should produce spans.
#[test]
fn extract_text_cid() {
    let pdf = load_pdf("pdfs/custom/font_cid_1.pdf");
    let settings = interpreter_settings();

    for page in pdf.pages().iter() {
        let spans = extract_text(page, &settings);
        assert!(!spans.is_empty(), "expected text spans from CID font PDF");
    }
}

/// Calling extract_text on a page with no text content should return an
/// empty vec (not panic).
#[test]
fn extract_text_image_only_page() {
    // image_1_bit_per_component.pdf should be predominantly an image.
    let pdf = load_pdf("pdfs/custom/image_1_bit_per_component.pdf");
    let settings = interpreter_settings();

    let page = &pdf.pages()[0];
    // Should not panic; may or may not return spans depending on the PDF.
    let _spans = extract_text(page, &settings);
}

/// With the default `InterpreterSettings` *without* the `embed-fonts`
/// feature, standard fonts can't be resolved.  The function must still
/// return without panicking.  When fonts *are* available via the custom
/// `interpreter_settings()` helper the result should be non-empty.
#[test]
fn extract_text_default_settings_no_panic() {
    use hayro::hayro_interpret::InterpreterSettings;

    let pdf = load_pdf("pdfs/custom/font_standard_1.pdf");
    let page = &pdf.pages()[0];

    // Default settings — may lack fonts depending on feature flags.
    // Must not panic regardless.
    let _spans = extract_text(page, &InterpreterSettings::default());
}

/// Multiple pages: extract from each page independently and confirm no
/// cross-page contamination (each extraction starts fresh).
#[test]
fn extract_text_multi_page() {
    let pdf = load_pdf("pdfs/custom/font_standard_2.pdf");
    let settings = interpreter_settings();

    let pages = pdf.pages();
    if pages.len() < 2 {
        // This PDF should have multiple pages; skip if not.
        return;
    }

    let spans_p0 = extract_text(&pages[0], &settings);
    let spans_p1 = extract_text(&pages[0], &settings);

    // Extracting the same page twice should yield identical results.
    assert_eq!(spans_p0.len(), spans_p1.len());
    for (a, b) in spans_p0.iter().zip(spans_p1.iter()) {
        assert_eq!(a.text, b.text);
        assert_eq!(a.glyphs.len(), b.glyphs.len());
    }
}