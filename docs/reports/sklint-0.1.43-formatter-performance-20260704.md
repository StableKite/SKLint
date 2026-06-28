# SKLint 0.1.43 — formatter performance

## Root causes

The formatter previously performed structural `SK505` and `SK509` moves one block per full analysis round. A class with many independent dependency-order violations therefore reran every enabled rule after every single method move.

Safe text fixes had a second quadratic path: every replacement converted line/column coordinates to byte offsets by scanning the source from the beginning again.

## Changes

- Batch all independent moves in the currently selected structural category (`SK509`, method `SK505`, or top-level `SK505`) before the next full analysis round.
- Preserve suppression filtering by deriving structural moves only from diagnostics that survived project and local suppressions.
- Keep cycle handling and stable source order for cyclic dependencies.
- Track original line identities while blocks move, so decorators and complete definition bodies remain intact without reparsing after every move.
- Build one source line index per ordinary-fix round and calculate all byte ranges before applying replacements from bottom to top.
- Add regression tests proving that 40 method moves, three special-method moves, and two top-level moves are each completed in one structural stage.

## Measurements

Release binaries were measured in the same Linux container.

| Workload | 0.1.42 | 0.1.43 |
|---|---:|---:|
| One file, 200 independent method dependency moves | 7.99 s | 0.14 s |
| 20 files, 50 moves per file | 3.28 s | 0.24 s |
| 351 files, 50 moves per file | did not finish in 180 s | 3.41 s |
| 10,000 lines with ordinary safe fixes | 4.11 s | 1.30 s |

The available `event.py` fixture produced byte-identical output with 0.1.42 and 0.1.43 and compiled successfully after formatting.
