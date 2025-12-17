# Test Input Implementation Order

Based on actual segment analysis of test files, ordered from simplest to hardest.

## 3. Stripes
**Segments:** `PageInfo, ImmLossGen, EOS, ..., EOP`

```
bitmap-stripe.jbig2
bitmap-stripe-single.jbig2
bitmap-stripe-single-no-end-of-stripe.jbig2
bitmap-stripe-last-implicit.jbig2
bitmap-stripe-initially-unknown-height.jbig2
```

**Why:** Adds EndOfStripe handling, otherwise same generic region decoder.

---

## 5. Refinement Regions
**Segments:** `PageInfo, IntGen, ImmRefine, EOP` or `PageInfo, ImmGen, ImmRefine, EOP`

```
bitmap-refine.jbig2
bitmap-refine-customat.jbig2
bitmap-refine-customat-tpgron.jbig2
bitmap-refine-lossless.jbig2
bitmap-refine-template1.jbig2
bitmap-refine-tpgron.jbig2
bitmap-refine-template1-tpgron.jbig2
bitmap-refine-page.jbig2
bitmap-refine-page-subrect.jbig2
bitmap-refine-refine.jbig2                          (IntGen, IntRefine, ImmRefine)
bitmap-trailing-7fff-stripped-harder-refine.jbig2
```

**Why:** New context model using reference bitmap. Requires generic region first.

---

## 6. Halftone Regions
**Segments:** `PageInfo, PatDict, ImmLossHalf/ImmHalf, EOP`

```
bitmap-halftone.jbig2
bitmap-halftone-template1.jbig2
bitmap-halftone-template2.jbig2
bitmap-halftone-template3.jbig2
bitmap-halftone-10bpp.jbig2
bitmap-halftone-10bpp-mmr.jbig2
bitmap-halftone-refine.jbig2                        (PatDict, IntHalf, ImmRefine)
```

**Why:** Requires PatternDictionary + grayscale index decoding. Can be done in parallel with symbols.

---

## 7. Composite + Refinement/Halftone
**Segments:** Mixed with compositing

```
bitmap-composite-and-xnor-refine.jbig2
bitmap-composite-or-xor-replace-refine.jbig2
bitmap-composite-and-xnor-halftone.jbig2
bitmap-composite-or-xor-replace-halftone.jbig2
```

---

## 8. Symbol Dictionary + Text Region (Basic)
**Segments:** `PageInfo, SymDict, ImmLossText/ImmText, EOP`

```
bitmap-symbol.jbig2
bitmap-symbol-textbottomleft.jbig2
bitmap-symbol-textbottomright.jbig2
bitmap-symbol-texttopright.jbig2
bitmap-symbol-texttranspose.jbig2
bitmap-symbol-textbottomlefttranspose.jbig2
bitmap-symbol-textbottomrighttranspose.jbig2
bitmap-symbol-texttoprighttranspose.jbig2
bitmap-symbol-negative-sbdsoffset.jbig2
bitmap-symbol-textrefine.jbig2
bitmap-symbol-textrefine-customat.jbig2
bitmap-symbol-textrefine-negative-delta-width.jbig2
```

**Why:** Major milestone - symbol dictionary + text region placement.

---

## 9. Symbol + Composite
**Segments:** `PageInfo, SymDict, ImmLossText, ImmLossText, ..., EOP`

```
bitmap-composite-and-xnor-text.jbig2
bitmap-composite-or-xor-replace-text.jbig2
bitmap-symbol-textcomposite.jbig2
```

---

## 10. Multiple Symbol Dictionaries
**Segments:** `PageInfo, SymDict, SymDict, ..., ImmText, EOP`

```
bitmap-symbol-context-reuse.jbig2
bitmap-symbol-manyrefs.jbig2
bitmap-symbol-symbolrefineone.jbig2
bitmap-symbol-symbolrefineone-customat.jbig2
bitmap-symbol-symbolrefineone-template1.jbig2
bitmap-symbol-symbolrefineseveral.jbig2
bitmap-symbol-refine.jbig2                          (SymDict, IntText, ImmRefine)
```

---

## 11. Symbol + Huffman (Standard Tables)
**Segments:** `PageInfo, SymDict, ImmText, EOP` (but uses Huffman internally)

```
bitmap-symbol-symhuff-texthuff.jbig2
bitmap-symbol-symhuff-texthuffB10B13.jbig2
bitmap-symbol-symhuffB5B3-texthuffB7B9B12.jbig2
bitmap-symbol-symhuffuncompressed-texthuff.jbig2
bitmap-symbol-symhuffrefineone.jbig2
bitmap-symbol-symhuffrefineseveral.jbig2
bitmap-symbol-texthuffrefine.jbig2
bitmap-symbol-texthuffrefineB15.jbig2
```

---

## 12. Symbol + Custom Huffman Tables (Hardest)
**Segments:** `PageInfo, Tables, ..., SymDict, Tables, ..., ImmLossText, EOP`

```
bitmap-symbol-symhuffcustom-texthuffcustom.jbig2
bitmap-symbol-texthuffrefinecustom.jbig2
bitmap-symbol-texthuffrefinecustomdims.jbig2
bitmap-symbol-texthuffrefinecustompos.jbig2
bitmap-symbol-texthuffrefinecustomposdims.jbig2
bitmap-symbol-texthuffrefinecustomsize.jbig2
```

---

## 13. Multi-Page (Integration Test)

```
annex-h.jbig2                                       (3 pages with SymDict, Text, Generic, Halftone)
```

---

## Errors (Need Special Handling)

```
bitmap-initially-unknown-size.jbig2                 (unknown segment data length - needs end marker scanning)
bitmap-p32-eof.jbig2                                (unknown segment type)
```

---

## Summary

Start with **Group 1** (`bitmap.jbig2`, `bitmap-mmr.jbig2`) - they have identical segment structure, just different encoding flags inside the generic region data.

### Suggested Implementation Order

1. **Generic region (template 0)** - core arithmetic + context
2. **MMR variant** - if CCITT Group 4 already works
3. **Other templates + TPGDON + custom AT** - variations on generic
4. **Stripes** - segment handling
5. **Composite operations** - region combination operators
6. **Refinement** - new context model
7. **Halftone** - can be done in parallel with symbols
8. **Symbol dictionary** - major milestone
9. **Text regions** - uses symbols
10. **Huffman variants** - alternative coding
11. **Custom Huffman tables** - final boss
