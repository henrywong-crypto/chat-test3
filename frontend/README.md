# Frontend

## Source files

- `src/terminal.js` — xterm terminal init; reads config from `#app-config` data attributes
- `src/file_manager.js` — file browser panel; reads config from `#app-config` data attributes
- `src/styles.css` — all app styles; imports xterm CSS via `@xterm/xterm`

## Building

```sh
npm install
npm run build
```

Watch mode (rebuilds on save):

```sh
npm run watch
```

## How it works

esbuild bundles each JS entry point (including xterm from node_modules) into `dist/`.
The Rust server embeds the `dist/` files via `include_bytes!` at compile time,
so the binary is self-contained with no CDN dependencies.

After running `npm run build`, rebuild the Rust server with `cargo build`.
