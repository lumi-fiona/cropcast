# 0002 — Ship SudoVDA signed with cropcast's own certificate

Status: accepted, 2026-10-17 ([Driver first-run flow](https://github.com/lumi-fiona/cropcast/issues/14))

## Context

SudoVDA has no release binaries, and the only signed build (inside Apollo) is signed with Apollo's self-signed certificate. A self-signed driver installs only after its certificate is trusted machine-wide, as a root and a trusted publisher, so whoever holds that key can sign anything the PC will trust. Proper signing (attestation) needs a paid EV certificate and a Partner Center account, which is too much for a single-user portfolio app.

## Decision

- CI builds SudoVDA from a pinned commit with the WDK and signs its catalog with **cropcast's own** code-signing-only certificate, valid 10 years. The private key exists only as a GitHub secret.
- The Driver is embedded in the portable exe. The first time the Virtual monitor is turned on without a matching Driver, cropcast re-launches itself elevated (one UAC prompt), trusts its certificate, installs, and cleans up its temp files. A protocol-version mismatch triggers the same reinstall.
- Declined or failed installs leave nothing behind and the Virtual monitor stays off; the next turn-on asks again.
- **Remove driver…** in the tray (and `--uninstall`) removes the device, the package and the certificate.
- If Apollo's SudoVDA is present without the Apollo app, the install replaces it and removes Apollo's certificate.

## Rejected

- **Shipping Apollo's signed package:** zero build work, but every PC would root-trust a third party's key.
- **Attestation signing:** EV certificate and identity verification for one user.
- **A zip with a driver folder:** the map promises one portable exe.

## Consequences

- If cropcast's key leaks, every PC with the Driver trusts code signed by the thief until the certificate is removed. The key must never leave CI secrets.
- **[test]** An installed Driver should keep loading after the certificate expires; reinstalling after expiry needs a new certificate.
