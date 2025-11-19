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
    This will trigger the Homebrew workflow, which creates a PR on the autobib/homebrew-autobib tap repo to update the formula, and in turn starts a workflow on that repo to test and bottle the updated formula.
10. Wait for the bottling workflow to complete on the tap repo PR, then apply the `pr-pull` label.
    The label triggers another workflow on the tap repo that pushes the formula bump and new bottle hashes onto the main branch, and closes the PR.

## Automation

A convenience script has been created to automate steps 1-5.
The script can be found in [`scripts/release.sh`](scripts/release.sh) and can be run with
```sh
./scripts/release.sh {major, minor, patch, rc, beta, alpha}
```
This script requires that the following tools are installed

- [`sed`](https://www.gnu.org/software/sed/) (GNU-compatible)
- [`cargo-edit`](https://crates.io/crates/cargo-edit)
