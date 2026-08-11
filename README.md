# SocialFlow

Photography-first, local-first macOS campaign planning built with Tauri 2, React, TypeScript, Rust and SQLite.

## Development

```sh
npm install
npm run tauri dev
```

## Validation

```sh
npm run build
npm test
cd src-tauri && cargo fmt --check && cargo test && cargo clippy -- -D warnings
```

## Production

```sh
npm run tauri build
```

Original photographs are indexed in place. SocialFlow never modifies, moves, renames or deletes them.
