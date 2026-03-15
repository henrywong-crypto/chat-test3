# Frontend

React + TypeScript single-page app bundled with esbuild.

## Building

```sh
npm install
npm run build   # outputs dist/app.js and dist/styles.css
npm run watch   # rebuild on save
```

After building, rebuild the Rust server with `cargo build` — it embeds `dist/` via `include_bytes!`.

## Testing

```sh
npx playwright test          # run all tests
npx playwright test --ui     # interactive mode
```

Tests live in `tests/`. The `tests/helpers/setup.ts` helper mounts the app in a mocked browser environment (no real server needed).

## Source layout

```
src/
  index.tsx              # entry point
  types.ts               # shared TypeScript types
  contexts/
    SseContext.tsx        # EventSource connection, API calls
  hooks/
    useChatState.ts       # per-session message/streaming state
    useSseHandlers.ts     # routes SSE events into chat state
  components/
    App.tsx               # root layout, session list
    ChatInterface.tsx     # chat pane, send/stop logic
    Sidebar.tsx           # session list
    ChatMessagesPane.tsx  # message bubbles
    ChatComposer.tsx      # textarea + send button
    ClaudeStatus.tsx      # streaming status bar
    Terminal.tsx          # xterm terminal tab
    FileManager.tsx       # file browser tab
    ...
  utils/
    transcript.ts         # rebuild ChatMessage[] from server transcript
```

## Key concepts

**Session lifecycle** — when a user sends from blank state, `handleSend` generates a `crypto.randomUUID()` as a pending session ID. A "New chat…" placeholder appears in the sidebar immediately. On the `done` SSE event, `loadHistory()` replaces the placeholder with the real session returned by the server.

**SSE event flow** — `SseContext` opens an `EventSource` and pushes raw events into `eventQueueRef`. `useSseHandlers` drains the queue on each `eventSeq` tick and updates `useChatState`.

**No CDN** — the Rust server embeds the compiled `dist/` files at compile time; the app runs fully offline.
