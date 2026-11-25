## Summary

<!-- Explain what this PR changes and why. Link any related issue (#...). -->

## Area

<!-- Check all that apply -->
- [ ] Desktop UI (`apps/m3-nsh`)
- [ ] Core Engine / Lakehouse / Research (`crates/m3-core`)
- [ ] CLI (`crates/m3-cli`)
- [ ] Website (`website`)
- [ ] Docs / CI / Packaging

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --locked --all-targets -- -D warnings`
- [ ] `cargo test --workspace --locked -j 4`
- [ ] Zero production `.unwrap()` / `.expect()` under `crates/*/src` and `apps/*/src`
- [ ] `pnpm --dir website build` (if `website/` was modified)
