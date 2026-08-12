# Security policy

## Supported versions

Security fixes are provided for the latest published BurnSubs release.

## Reporting a vulnerability

Please do not open a public issue for a suspected vulnerability. Use GitHub's **Report a
vulnerability** form in the repository's Security tab. Include the affected version, operating
system, reproduction steps, impact, and any suggested mitigation.

If private vulnerability reporting is not enabled yet, the repository owner should enable it under
**Settings > Security > Code security and analysis > Private vulnerability reporting** before the
first public release.

BurnSubs invokes local FFmpeg executables and processes user-selected paths. Reports involving
command construction, unsafe path handling, output replacement, temporary files, or untrusted media
are especially useful.
