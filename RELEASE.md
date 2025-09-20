# Publishing a new release

1. Modify `Cargo.toml` version to the desired version `X.Y.Z`.
2. Run `cargo update`.
3. Rename `docs/changelog/next.md` to `docs/changelog/vX.Y.Z.md`.
4. Change the header of `docs/changelog/vX.Y.Z.md` to `# Version X.Y.Z (<date +%Y-%m-%d>)`.
   For example, `# Version 0.2.0 (2024-11-26)`.
5. Create a new branch called `create-release-vX.Y.Z`, add all, and commit changes.
6. Submit a PR with a title like "Release vX.Y.Z".
7. Wait for CI to succeed, merge the PR.
8. Create (and push) a new tag on the release commit of the form `vX.Y.Z`.
   The release workflow will automatically compile the binaries and publish to `crates.io`.
   It will also create a draft release with all of the release binaries.
9. Go to the 'releases' tab on GitHub and publish the draft release.


## Automation

A convenience step has been created to automate steps 1-5.
The script can be find in [`scripts/release.sh`](scripts/release.sh) and can be run with
```
./scripts/release.sh {major, minor, patch, rc, beta, alpha}
```
This script requires that the following tools are installed
- [`sed`](https://www.gnu.org/software/sed/) (GNU-compatible)
- [`cargo-edit`](https://crates.io/crates/cargo-edit)
- [`yq`](https://mikefarah.gitbook.io/yq)
