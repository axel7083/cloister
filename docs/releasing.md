# Releasing

1. Bump the version in `Cargo.toml` (`[workspace.package]`), `meson.build`
   and `packaging/cloister.spec` (`Version:` and a `%changelog` entry).
   `release.yaml` refuses a tag that does not match all three.
2. Go through the manual checklist in [testing.md](testing.md).
3. Tag with an annotated tag; its message becomes the release notes:
   ```sh
   git tag -a v0.2.0 -m "Cloister 0.2.0

   - what changed"
   git push origin v0.2.0
   ```
   A tag with a suffix (`v0.2.0-rc1`) publishes a **pre-release**.

`release.yaml` then re-runs pr-check and the end-to-end tests **on the tag**,
and publishes exactly the artifacts that were tested:

| File | Content |
|---|---|
| `cloister-X.Y.Z.tar.gz` | source |
| `cloister-X.Y.Z-vendor.tar.xz` | vendored crates (offline builds) |
| `cloister-X.Y.Z-1.fc45.src.rpm`, `cloister-X.Y.Z-1.fc45.x86_64.rpm` | Fedora packages |
| `SHA256SUMS` | checksums |

Every file has a GitHub build provenance attestation:
```sh
gh attestation verify cloister-X.Y.Z-1.fc45.x86_64.rpm --repo axel7083/cloister
```

## COPR

Publishing the GitHub release makes [Packit](https://packit.dev) build the
package for every Fedora release in
[axel7083/cloister](https://copr.fedorainfracloud.org/coprs/axel7083/cloister/)
(the `release` job in `.packit.yaml`); users get it with `dnf upgrade`. Check
the build there before announcing the release. Pull requests get a test build
in Packit's own COPR namespace, shown as checks on the PR.
