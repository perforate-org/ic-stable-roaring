# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | Yes       |
| < 0.1.0 | No        |

## Reporting a Vulnerability

Please report suspected vulnerabilities privately to
[yota@perforate.org](mailto:yota@perforate.org). Do not open a public issue or publish an exploit
before the maintainer has had a reasonable opportunity to investigate and coordinate a fix.

Include the affected version, a minimal reproduction, expected and observed behavior, and any
stable-memory layout or upgrade conditions needed to trigger the issue. Reports affecting data
integrity, initialization/recovery, or denial of service are especially useful.

The maintainer will acknowledge receipt, assess the report, and coordinate a fix and disclosure
timeline with the reporter. There is no public bug-bounty program.

## Stable-memory trust boundary

`RoaringBitmap::init` is designed for valid, isolated stable memory owned by the caller. It
validates the reachable header, snapshot, and contiguous journal prefix, but intentionally does
not scan unreachable bytes after the first empty journal slot. Applications that expose stable
memory to untrusted mutation or require an offline corruption audit need an additional integrity
mechanism outside this crate.
