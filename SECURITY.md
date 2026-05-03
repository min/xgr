# Security Policy

## Supported Versions

`XcodeGenRust` is pre-1.0. Report security issues against the current `main` branch.

## Reporting a Vulnerability

Please do not open a public issue for a suspected vulnerability. Use GitHub private vulnerability reporting if it is enabled for the repository, or contact the maintainer privately through GitHub so the issue can be investigated before disclosure.

## Scope

Project specs are treated as trusted configuration. `xgr` is not a sandbox for untrusted specs: specs can reference files on disk and choose output paths for generated project artifacts. Run it in a disposable workspace when evaluating third-party specs.
