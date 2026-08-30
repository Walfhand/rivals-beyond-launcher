# Code signing policy

Rivals Beyond publishes Windows launcher releases from this public repository through a reproducible GitHub Actions workflow.

**Requested signing service:** Free code signing provided by SignPath.io, certificate by SignPath Foundation.

Authenticode signing is pending SignPath Foundation approval. Until approval, release pages must describe Windows installers as unsigned and must not claim a verified publisher.

## Team roles

- Committer and reviewer: [Walfhand](https://github.com/Walfhand)
- Signing approver: [Walfhand](https://github.com/Walfhand)

Changes from outside maintainers require review before merge. Every signing request requires manual approval by the signing approver.

## Release path

1. GitHub Actions checks out a tagged source revision on a GitHub-hosted Windows runner.
2. Rust and Python tests run before packaging.
3. Tauri builds the NSIS installer and its updater signature.
4. Once SignPath integration is approved, the unsigned artifact is submitted for Authenticode signing and manually approved.
5. The workflow verifies both Authenticode and the Tauri updater signature before publication.
6. Versioned artifacts are immutable; the automatic `stable.json` feed is published last.

## Privacy

The launcher privacy policy is documented in [PRIVACY.md](PRIVACY.md).
