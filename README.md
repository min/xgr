# oxidegen

This is a Rust port scaffold for [`yonaskolb/xcodegen`](https://github.com/yonaskolb/xcodegen).

Implemented now:

- XcodeGen-style YAML/JSON `project.yml` loading.
- `include` resolution with `relativePaths`, `enable`, duplicate include protection, additive merging, and `:REPLACE`.
- Variable expansion for `${name}` values supplied through the CLI.
- Target and scheme template merging.
- Multi-platform target expansion.
- Broad typed model coverage for the top-level project and targets while retaining the raw resolved spec for forward compatibility.
- A deterministic PBX graph writer for `.xcodeproj` output.
- Integration tests pointed at the upstream fixture specs cloned into `upstream-xcodegen`.

Still to complete for full parity:

- A PBX graph writer matching XcodeGen/XcodeProj output byte-for-byte.
- Complete source generation, scheme generation, breakpoint generation, cache files, and validations.
- Golden fixture assertions currently exist as an ignored test until the PBX writer is complete.

After cloning, initialize the upstream fixture submodule:

```sh
git submodule update --init --recursive
```

Useful commands once a Rust toolchain is installed:

```sh
cargo test
cargo run -- validate --spec upstream-xcodegen/Tests/Fixtures/TestProject/project.yml
cargo run -- dump --spec upstream-xcodegen/Tests/Fixtures/TestProject/project.yml
cargo run -- generate --spec upstream-xcodegen/Tests/Fixtures/SPM/project.yml
```
