# APC browserd runtime

Agent Personal Computer integrates browserd as a pinned submodule at
`apc-browserd/third_party/browserd`.

The compositor does not start browserd during boot. Browserd is launched lazily
from the first `browser_*` MCP tool call that needs a real browser process. The
compositor starts browserd in Chromium GUI mode with a private Unix socket MCP
transport and forwards browser MCP calls over that socket.

## Build

```sh
apc-browserd/scripts/build.sh aarch64
```

The build script uses the browserd checkout and preserves Chromium source and
`chromium/src/out/Release` caches. The packaged runtime is written to:

```text
apc-browserd/out/aarch64/
```

To package an already-built checkout without compiling:

```sh
APC_BROWSERD_SKIP_BUILD=1 apc-browserd/scripts/build.sh aarch64
```
