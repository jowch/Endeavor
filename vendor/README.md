# Vendored crates

## gpui-pre 0.3.6

GPUI, as published on crates.io, with a fixed line wrapper. `Cargo.toml`'s
`[patch.crates-io]` builds the app against this copy instead of the
registry's. Everything else is the published 0.3.6 source, with `examples/`
and the registry's own `Cargo.lock` left out (and the `[[example]]` entries
removed from `Cargo.toml` with them).

The only changed file is `src/text_system/line_wrapper.rs`. Its fixes keep a
closing mark from starting a chat line alone:

- A character after an inline element (a code span, which gpui-base measures
  as one element) saw the space *before* the element as the previous
  character, so a `.` after `` `code` `` became a break.
- After a line wrapped, the next character saw the character before the one
  that overflowed, with the same effect.
- `?` was a break before it (meant for URLs). It is now a break after it, as
  `/` is: UAX #14 LB13 never breaks before them.
- When a word or code span is as wide as the line and is followed by a `.`,
  `,`, `)` and the like, there is nowhere to break but before the mark. The
  mark now hangs a few pixels past the line's end instead, as hanging
  punctuation does in print.

The tests are in the same file (`test_wrap_line_keeps_closing_marks_with_their_word`).
They don't run with the app's `cargo test`. `scripts/test-vendored-gpui.sh`
runs them: it copies this folder out of the tree, stubs the image and fonts
that the crate's other tests `include_bytes!` (the published crate leaves them
out), and runs the wrapper's tests.

CJK closing marks (`。，、）」`) still break before themselves, as before; the
hanging rule covers the Latin marks listed in `hangs_at_line_end`. At most
two marks hang, and only right after a word, so a run of `!!!!` or `....`
still wraps.

To compare with the registry:

```sh
diff -r ~/.cargo/registry/src/*/gpui-pre-0.3.6/src vendor/gpui-pre/src   # after cargo fetch on main
```

Drop this copy when GPUI is upgraded. GPUI 0.3.11 (gpui-kit 0.9) rewrites
the wrapper on ICU's UAX #14 segmenter, which fixes the first three; it still
breaks before the mark in the last case, so either carry that fix forward or
get it taken upstream.
