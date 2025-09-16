# Publishing a new release

1. Modify `Cargo.toml` version to the desired version `X.Y.Z`.
2. Rename `docs/changelog/next.md` to `docs/changelog/vX.Y.Z.md`.
3. Change the header of `docs/changelog/vX.Y.Z.md` to `# Version X.Y.Z (`date`)`.
   For example, `# Version 0.2.0 (2024-11-26)`.
3. Add all, commit changes, and submit a PR with a title like "Release vX.Y.Z".
4. Wait for CI to succeed, merge the PR.
5. Create (and push) a new tag on the release commit of the form `vX.Y.Z`.
   The release workflow will automatically compile the binaries and publish to `crates.io`.
   It will also create a draft release with all of the release binaries.
6. Go to the 'releases' tab and publish on GitHub and publish the draft release.
