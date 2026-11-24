# FR-REM05: F2 freedoms of a WebAssembly mod in a real game

Branch `yoake-free/rem05-f2check` (worktree `horizon-rem05`, from `yoake-free/merge` 38dfe430e). Goal: show in a live
offscreen game that a WASM mod can write files, make an HTTP POST and use a TCP socket, and that a worker's blocking
network calls leave the game's tick running.

## What was added

- `crates/yoake-wasm-sdk/examples/f2-check/`: the test mod. `yak.toml` declares a server module
  (`bin/wasm/f2-check-server.wasm`) and a worker (`[wasm] worker = "bin/wasm/f2-check-worker.wasm"`).
  - `server/`: the main instance. In `init` it writes `/data/x.txt` and reads it back, and writes
    `/game/f2-check-game.txt`. It logs `f2-check files data=ok game=ok`.
  - `worker/`: a `worker-mod` component on its own thread. It polls `/data/jobs.txt` every 50 ms and answers each job
    on the game log: `post <url> <body>` (wasi:http POST, with `content-length`), `tcp <host:port> <text>` (TCP, echo
    read to EOF), `quit`.
- `tools/f2_e2e.py`: starts one offscreen game (`--offscreen --no-focus --no-pause`, through `horizon-ctl`), installs the
  mod as an installed package (`yoake-mods.txt`, `yoake-mods/f2-check/0.1.0/`), starts an HTTP server and a TCP echo server
  on 127.0.0.1 (in threads), runs the checks and writes `.local/f2-e2e/results.json` plus the game log and a screenshot.

## Results (`.\hz-py.cmd tools\f2_e2e.py --bin-dir target\debug --wasm ... --worker ...`, debug build, Windows)

| check | result | detail |
|---|---|---|
| files | pass | main instance wrote and read back `/data/x.txt`; `/game/f2-check-game.txt` is in the game folder |
| http_post | pass | the server got `hello-f2` on `/echo`; the mod logged `status=200 reply=got:hello-f2` |
| tcp_echo | pass | the echo server got `hello-tcp` and the mod logged `echo=hello-tcp` |
| nonblocking | pass | a POST to a path that waits 2 s on the server: the client tick advanced 42 during the 2.45 s wait; the `state` queries kept answering (longest gap 0.72 s, measured with `wait 2` between them); the reply `got:hello-slow` came back |
| https | not tested | needs external network: rustls uses webpki-roots, so a local self-signed server cannot be trusted |

The run was started only through `horizon-ctl`; no focus was taken. Other runs of horizon-client were not overlapping.

## Bugs found

None in the engine. Two problems were in the test code and were fixed there:
- the e2e's echo listener thread printed a traceback when the listener closed at the end of a run (now ignored);
- the first worker POST had no `content-length`, and the test server read an empty body (it did not decode chunked
  transfer). The guest now sends `content-length`; the test server also decodes chunked bodies. Whether the engine sends
  a chunked POST body correctly was **not verified** (not tested after the fix).

## What is left / 要確認

1. **Main instance to worker messaging is not run in the game.** The full server world (`crates/yoake-wasm/wit/yoake-full.wit`,
   `server-full`) does not import `worker`, so a server module cannot call `worker_send`/`recv` through its SDK. The worker
   here gets its jobs from `/data/jobs.txt` (written by the e2e script), not from the main instance. A worker-to-main reply
   path is therefore not exercised in a live game. Fixing this needs the `worker` import in the full server world (a WIT change
   to the 1.x baseline): an owner decision.
2. **HTTPS**: not tested (see above). Needs a trust path for a local certificate, or an external network test.
3. **Chunked POST bodies** from a mod (no `content-length`): not verified in the engine.
4. The "game thread never blocks" check measures the client tick (`state.ticks`), not the integrated server's own tick counter.
5. Only the debug build was run (`target/debug`); the release build and the two-client LAN case were not.
