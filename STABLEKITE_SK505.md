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
Unchanged-version pushes, manual dispatches and workflow reruns validate
without devpi access. The first run of a default-branch push can publish an
increased version. An unavailable previous version, a downgrade or a forced
release update fails closed.

CI uses only the write-only DEVPI_TOKEN, limited to the upload step. The
endpoint is https://devpi.stablekite.com/StableKite/index/ with certificate
verification and no redirect acceptance. The exact five wheel targets plus the
canonical sdist must belong to the current successful run. Wheel metadata,
complete RECORD hashes, native build provenance and an installed consumer are
checked before upload. A receipt records the start of the upload and the HTTP
acknowledgement, size and SHA-256 of each original. A failed or ambiguous upload
requires inspection before manual recovery.

The local collision and confirm commands can use a separately authorized read
credential. Confirmation downloads and checks the exact file names, sizes and
SHA-256 values. A successful CI upload receipt records that server bytes have
yet to be checked independently.

Offline tests exercise publication intent, all five dependencies, run and
checkout binding, credential separation, endpoint restrictions, native
provenance, incomplete artifacts and the write-only upload path.
