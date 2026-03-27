//! PDF structure element tags for marked-content sequences.
//!
//! These tag names appear in marked-content sequences (`BMC`/`BDC` operators)
//! and in the structure tree. This module covers the commonly encountered
//! standard types from PDF 1.7 (ISO 32000-1:2008): grouping elements,
//! headings, paragraphs, lists, tables, inline elements, illustration
//! elements, and ruby/warichu annotations. It also includes `Artifact`,
//! which is a marked-content tag (not a structure type) used to identify
//! page-level furniture such as headers, footers, and watermarks.
//!
//! PDF 2.0 (ISO 32000-2:2020) added several structure types not yet
//! represented here, including `DocumentFragment`, `Aside`, `Title`,
//! `FENote`, `Sub`, `Em`, and `Strong`. These will parse as
//! [`Other`](StructureTag::Other).
//!
//! Non-standard tags (including remapped tags via `RoleMap`) are also
//! represented by [`Other`](StructureTag::Other).

use core::fmt;

/// Defines [`StructureTag`] with `block` and `inline` groups.
///
/// Each variant name must be identical to its PDF tag string
/// (e.g. `BlockQuote` ↔ `"BlockQuote"`).  Placing a variant in `block`
/// or `inline` determines the return value of [`StructureTag::is_block_level`]
/// and [`StructureTag::is_inline`], so adding a new tag forces you to choose
/// a group at the declaration site.
macro_rules! structure_tags {
    (
        block {
            $(
                $(#[$block_meta:meta])*
                $block_variant:ident
            ),* $(,)?
        }
        inline {
            $(
                $(#[$inline_meta:meta])*
                $inline_variant:ident
            ),* $(,)?
        }
    ) => {
        /// A PDF structure element tag.
        ///
        /// Variants cover the commonly encountered standard structure types
        /// from PDF 1.7, plus `Artifact`. Tags that don't match a known
        /// variant are stored in [`Other`](Self::Other).
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum StructureTag {
            $(
                $(#[$block_meta])*
                $block_variant,
            )*
            $(
                $(#[$inline_meta])*
                $inline_variant,
            )*
            /// A tag name that does not match any standard structure type.
            Other(String),
        }

        impl StructureTag {
            /// Parse a structure tag from a raw byte slice (as found in
            /// `BMC`/`BDC` operands).
            pub fn from_bytes(tag: &[u8]) -> Self {
                if let Ok(s) = ::core::str::from_utf8(tag) {
                    match s {
                        $(stringify!($block_variant) => return Self::$block_variant,)*
                        $(stringify!($inline_variant) => return Self::$inline_variant,)*
                        _ => {}
                    }
                }
                Self::Other(String::from_utf8_lossy(tag).into_owned())
            }

            /// The tag name as it appears in the PDF content stream.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$block_variant => stringify!($block_variant),)*
                    $(Self::$inline_variant => stringify!($inline_variant),)*
                    Self::Other(s) => s.as_str(),
                }
            }

            /// Whether this tag represents a block-level structural element.
            ///
            /// Block-level tags delineate the document's major structural
            /// units (sections, headings, paragraphs, list items, table rows,
            /// figures, etc.).  The `Artifact` tag is included because
            /// artifact sequences wrap page-level furniture (headers, footers,
            /// watermarks) that acts as a block-level boundary for text
            /// reflow.
            pub fn is_block_level(&self) -> bool {
                matches!(self, $(Self::$block_variant)|*)
            }

            /// Whether this tag represents an inline-level structural element.
            pub fn is_inline(&self) -> bool {
                matches!(self, $(Self::$inline_variant)|*)
            }

            /// Whether this is a standard PDF structure tag (as opposed to
            /// [`Other`](Self::Other)).
            pub fn is_standard(&self) -> bool {
                !matches!(self, Self::Other(_))
            }

            /// Whether this is the [`Artifact`](Self::Artifact) tag.
            pub fn is_artifact(&self) -> bool {
                matches!(self, Self::Artifact)
            }
        }
    };
}

structure_tags! {
    block {
        // -- Grouping elements --------------------------------------------
        /// The root element of a document's structure tree.
        Document,
        /// A large division of a document.
        Part,
        /// An article (a relatively self-contained body of text).
        Art,
        /// A section.
        Sect,
        /// A generic block-level grouping element.
        Div,
        /// A block-level quotation.
        BlockQuote,
        /// A caption associated with a figure, table, etc.
        Caption,
        /// Table of contents.
        TOC,
        /// An individual entry in a table of contents.
        TOCI,
        /// An index.
        Index,

        // -- Paragraph-like elements --------------------------------------
        /// A generic heading (use H1–H6 when levels are known).
        H,
        /// Heading level 1.
        H1,
        /// Heading level 2.
        H2,
        /// Heading level 3.
        H3,
        /// Heading level 4.
        H4,
        /// Heading level 5.
        H5,
        /// Heading level 6.
        H6,
        /// A paragraph.
        P,

        // -- List elements ------------------------------------------------
        /// A list.
        L,
        /// A list item.
        LI,
        /// The label of a list item (e.g. bullet, number).
        Lbl,
        /// The body of a list item.
        LBody,

        // -- Table elements -----------------------------------------------
        /// A table.
        Table,
        /// A table row.
        TR,
        /// A table header cell.
        TH,
        /// A table data cell.
        TD,
        /// A table header row group.
        THead,
        /// A table body row group.
        TBody,
        /// A table footer row group.
        TFoot,

        // -- Illustration elements ----------------------------------------
        /// A figure (image, graphic, or other visual content).
        Figure,
        /// A mathematical formula.
        Formula,
        /// An interactive form widget.
        Form,

        // -- Special ------------------------------------------------------
        /// Content that is not part of the document's logical structure
        /// (e.g. page headers, footers, watermarks).
        Artifact,
    }
    inline {
        // -- Inline-level elements ----------------------------------------
        /// A generic inline-level grouping element.
        Span,
        /// An inline quotation.
        Quote,
        /// A note (e.g. footnote, endnote).
        Note,
        /// A cross-reference to another part of the document.
        Reference,
        /// A bibliographic entry.
        BibEntry,
        /// A fragment of computer code.
        Code,
        /// A hyperlink.
        Link,
        /// An annotation (other than a link).
        Annot,

        // -- Ruby / Warichu (CJK inline annotations) ----------------------
        /// A ruby annotation wrapper.
        Ruby,
        /// Ruby base text.
        RB,
        /// Ruby annotation text.
        RT,
        /// Ruby punctuation.
        RP,
        /// A warichu annotation wrapper.
        Warichu,
        /// Warichu text.
        WT,
        /// Warichu punctuation.
        WP,
    }
}

impl fmt::Display for StructureTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_standard_tags() {
        let tags = [
            StructureTag::Document,
            StructureTag::Part,
            StructureTag::Art,
            StructureTag::Sect,
            StructureTag::Div,
            StructureTag::BlockQuote,
            StructureTag::Caption,
            StructureTag::TOC,
            StructureTag::TOCI,
            StructureTag::Index,
            StructureTag::H,
            StructureTag::H1,
            StructureTag::H2,
            StructureTag::H3,
            StructureTag::H4,
            StructureTag::H5,
            StructureTag::H6,
            StructureTag::P,
            StructureTag::L,
            StructureTag::LI,
            StructureTag::Lbl,
            StructureTag::LBody,
            StructureTag::Table,
            StructureTag::TR,
            StructureTag::TH,
            StructureTag::TD,
            StructureTag::THead,
            StructureTag::TBody,
            StructureTag::TFoot,
            StructureTag::Span,
            StructureTag::Quote,
            StructureTag::Note,
            StructureTag::Reference,
            StructureTag::BibEntry,
            StructureTag::Code,
            StructureTag::Link,
            StructureTag::Annot,
            StructureTag::Figure,
            StructureTag::Formula,
            StructureTag::Form,
            StructureTag::Ruby,
            StructureTag::RB,
            StructureTag::RT,
            StructureTag::RP,
            StructureTag::Warichu,
            StructureTag::WT,
            StructureTag::WP,
            StructureTag::Artifact,
        ];

        for tag in &tags {
            let bytes = tag.as_str().as_bytes();
            let parsed = StructureTag::from_bytes(bytes);
            assert_eq!(&parsed, tag, "round-trip failed for {tag}");
            assert!(parsed.is_standard());
        }
    }

    #[test]
    fn non_standard_tag() {
        let tag = StructureTag::from_bytes(b"CustomTag");
        assert_eq!(tag, StructureTag::Other("CustomTag".into()));
        assert!(!tag.is_standard());
        assert!(!tag.is_block_level());
        assert!(!tag.is_inline());
        assert_eq!(tag.as_str(), "CustomTag");
        assert_eq!(tag.to_string(), "CustomTag");
    }

    #[test]
    fn block_vs_inline() {
        assert!(StructureTag::P.is_block_level());
        assert!(!StructureTag::P.is_inline());

        assert!(StructureTag::Span.is_inline());
        assert!(!StructureTag::Span.is_block_level());

        assert!(StructureTag::Artifact.is_block_level());
        assert!(StructureTag::Artifact.is_artifact());
        assert!(!StructureTag::Artifact.is_inline());
    }

    #[test]
    fn display_impl() {
        assert_eq!(format!("{}", StructureTag::H1), "H1");
        assert_eq!(format!("{}", StructureTag::BlockQuote), "BlockQuote");
        assert_eq!(
            format!("{}", StructureTag::Other("Foo".into())),
            "Foo"
        );
    }
}