# Fuzz testing mpeg2ts-reader

## Fuzz corpus

TODO: ❌ There isn't yet a mechanism for distributing the fuzz testing corpus data.  (It is not recommended to commit
the data along with the project source code, so options might be to set up a seperate repo, or find some other file /
object store from which to download the corpus.)

You can still run the fuzz test without a pre-existing corpus, but it will take the fuzzer several minutes to generate
a corpus from scratch.

## Running the fuzz test locally

From the repository root directory,

```
cargo +nightly fuzz coverage fuzz_target_1
```

(The fuzzer will continue testing random inputs until killed.)

## Checking the code coverage of the corpus

Having run the fuzz test as above and generated a test corpus, from the repository **root** directory, generate coverage
metadata for the codebase,

```
cargo +nightly fuzz coverage fuzz_target_1
```

Then, from within **this** directory (the `fuzz` subdirectory of the repository root),

```
cargo +nightly cov \
    -- show \
    target/x86_64-unknown-linux-gnu/release/fuzz_target_1 \
    --ignore-filename-regex=.cargo/ \
    --format=html \
    --instr-profile=coverage/fuzz_target_1/coverage.profdata \
    > target/fuzz-cov.html
```

Read the report in `fuzz-cov.html` to look for risky areas of the codebase that are not being touched by the fuzzer.