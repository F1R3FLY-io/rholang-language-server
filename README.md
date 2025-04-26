# Rholang Language Server

LSP-based Language Server for Rholang (Language Server Protocol).

## Dependencies

Clone [f1r3fly](https://github.com/f1R3FLY-io/f1r3fly) and compile `rnode`:

```shell
git clone https://github.com/f1R3FLY-io/f1r3fly.git
cd f1r3fly
export SBT_OPTS="-Xmx4g -Xss2m -Dsbt.supershell=false"
sbt clean bnfc:generate compile stage
# Optional: Add `rnode` to your $PATH:
export PATH="$PWD/node/target/universal/stage/bin:$PATH"
```

## Installing

Clone
[rholang-language-server](https://github.com/F1R3FLY-io/rholang-language-server)
and compile it:

```shell
git clone https://github.com/F1R3FLY-io/rholang-language-server.git
cd rholang-language-server
cargo build
# Optional: Add `rholang-language-server` to your $PATH:
export PATH="$PWD/target/debug:$PATH"
```

## Testing

1. From one terminal window, launch RNode in standalone mode: `rnode run -s`.
2. From another terminal window, `cd` into the root of `rholang-language-server`
   and run: `cargo test`.
   - This will spawn an instance of `rholang-language-server` and execute a
     sequence of tests against it.
   - `rholang-language-server` will communicate with the instance of RNode that
     is running in standalone mode.
