# hayro-jbig2 tests

Each test decodes a JBIG2 file from `streams/` and compares the result against an image in `references/`.

The format of these tests was heavily inspired by the [JBIG2 tests of SerenityOS](https://github.com/SerenityOS/serenity/tree/master/Tests/LibGfx/test-inputs/jbig2).

Before submitting new reference images, optimize them with [oxipng](https://github.com/oxipng/oxipng):

```bash
oxipng -o max --strip safe hayro-jbig2/tests/references/*.png
```
