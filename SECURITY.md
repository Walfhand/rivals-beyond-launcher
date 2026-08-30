# Security policy

Report vulnerabilities privately through [GitHub Security Advisories](https://github.com/Walfhand/rivals-beyond-launcher/security/advisories/new). Do not open a public issue for an unpatched vulnerability.

Official launcher updates use two independent checks:

- a Tauri updater signature authenticates the NSIS launcher release;
- signed client manifests and SHA-256 content addresses authenticate game-client files.

Private signing keys are never committed to this repository.
