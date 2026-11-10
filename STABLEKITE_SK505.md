# SK505 first binding and distribution checks

Version 0.1.52 checks the first actual top-level definition of each public name.
A call after the first overload declaration or function/class definition is valid.
A call before the first definition still produces one SK505 diagnostic.
Later overload declarations and redefinitions do not move the first binding.
The rule and its severity remain enabled.

The change adds three Rust regression groups. The original workspace has 493
passing tests on Windows and Linux, with formatting and Clippy warnings checked.
Fresh ordinary native wheel consumers on both systems pass the original public
positive/negative cases using the full project configuration. The Linux local
wheel carries its actual linux_x86_64 tag; only CI may establish the manylinux and
musllinux delivery claims.

All five existing platform jobs, including their audits, locked Rust gates,
native provenance, wheel checks, installed smoke tests and Windows VSIX/sdist
steps, are preserved. The sixth job records an explicit publication plan.
Unchanged-version pushes validate without authentication or publication.
Manual runs validate by default. Publication requires the exact default branch
and either a verified version change or an explicit manual publish input.
An unavailable or inconsistent previous push version fails closed.

Read authentication and collision checks use only DEVPI_READ_TOKEN. Upload uses
only DEVPI_TOKEN. The endpoint is
https://devpi.stablekite.com/StableKite/index/ with certificate verification and
no redirect acceptance. The exact five wheel targets plus the canonical sdist
must belong to the current successful run. Wheel metadata, complete RECORD
hashes, native build provenance and a fresh installed consumer are checked before
upload. Each original artifact receives one upload attempt; server file names,
sizes and downloaded SHA256 values must then match the qualified originals.
Missing credentials or a failed gate do not authorize a fallback publisher.

Offline failure controls exercise publication intent, all five dependencies,
run/head binding, previous-version validation, credential separation, TLS and
endpoint restrictions, authenticated absence, version collisions, native
provenance and corrupt or incomplete artifacts. They make no production calls.
