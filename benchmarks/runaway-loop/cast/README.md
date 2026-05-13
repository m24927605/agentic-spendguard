# Asciinema cast — runaway-loop benchmark demo

`runaway-loop.cast` is a ~10-second asciinema recording of the
benchmark's runner-and-analyzer phase (Docker build is excluded for
brevity). Each runner's JSON self-report appears as it lands, then
the analyzer's final summary table.

## Watch it

```bash
asciinema play cast/runaway-loop.cast
```

## Convert to GIF for embedding in docs / launch posts

```bash
brew install agg                                          # macOS
agg cast/runaway-loop.cast cast/runaway-loop.gif          # ~12s GIF
```

`agg` is the official asciinema → GIF converter
([github.com/asciinema/agg](https://github.com/asciinema/agg)).

## Re-record

```bash
bash cast/record.sh
```

Pre-builds images so the recorded portion is just the runners +
analyzer (~10 seconds). Tears down volumes after.

## Why we ship the cast file, not the GIF

- The cast is text (JSONL events) → diff-friendly, ~2.5 KB.
- The GIF is a binary asset that bloats the repo and gets stale.
- Anyone who wants the GIF for a post can regenerate it in two
  commands above.
