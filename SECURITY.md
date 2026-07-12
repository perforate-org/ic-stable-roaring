# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | Yes       |
| < 0.1.0 | No        |

## Reporting a Vulnerability

The preferred channel is GitHub Private Vulnerability Reporting, when it is enabled for this
repository. Open the repository's **Security** tab and choose **Report a vulnerability**. Do not
include vulnerability details in a public issue or pull request.

If Private Vulnerability Reporting is unavailable, report suspected vulnerabilities privately to
[yota@perforate.org](mailto:yota@perforate.org). If neither channel is available, contact the
repository owner through GitHub without including sensitive details and request a private
reporting channel.

Include the affected version, a minimal reproduction, expected and observed behavior, and any
stable-memory layout or upgrade conditions needed to trigger the issue. Reports affecting data
integrity, initialization/recovery, or denial of service are especially useful.

The maintainer will acknowledge receipt within five business days and aims to complete an initial
triage within ten business days. These are targets rather than guarantees.

We use coordinated disclosure. We agree on the affected versions, remediation plan, release date,
and public advisory timing with the reporter. Unless active exploitation or another circumstance
requires a different schedule, the target disclosure window is 90 days after acknowledgement.
Please do not publish an exploit or detailed report before the agreed disclosure date.

Security fixes are released for the latest supported `0.1.x` version. Backports to older `0.1.x`
releases are considered case by case, based on severity, compatibility risk, and whether the fix
can be safely applied without a layout migration. There is no public bug-bounty program.

## Stable-memory trust boundary

`RoaringBitmap::init` is designed for valid, isolated stable memory owned by the caller. It
validates the reachable header, snapshot, and contiguous journal prefix, but intentionally does
not scan unreachable bytes after the first empty journal slot. Applications that expose stable
memory to untrusted mutation or require an offline corruption audit need an additional integrity
mechanism outside this crate.
