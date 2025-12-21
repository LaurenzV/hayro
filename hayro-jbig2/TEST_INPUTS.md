# Test Input Implementation Order

## 8. Symbol Dictionary + Text Region (Basic)
**Segments:** `PageInfo, SymDict, ImmLossText/ImmText, EOP`

```
bitmap-symbol-textrefine.jbig2
bitmap-symbol-textrefine-customat.jbig2
bitmap-symbol-textrefine-negative-delta-width.jbig2
```

**Why:** Major milestone - symbol dictionary + text region placement.

---

## 10. Multiple Symbol Dictionaries
**Segments:** `PageInfo, SymDict, SymDict, ..., ImmText, EOP`

```
bitmap-symbol-context-reuse.jbig2
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
