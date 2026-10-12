# hayro-ccitt tests

Each test decodes a raw CCITT stream from `streams/` and compares the result against an image in `references/`.

Before submitting new reference images, optimize them with [oxipng](https://github.com/oxipng/oxipng):

```bash
oxipng -o max --strip safe hayro-ccitt/tests/references/*.png
```
