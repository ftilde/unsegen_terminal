# unsegen_terminal

[![](https://img.shields.io/crates/v/unsegen_terminal.svg)](https://crates.io/crates/unsegen_terminal/)
[![](https://docs.rs/unsegen_terminal/badge.svg)](https://docs.rs/unsegen_terminal/)

`unsegen_terminal` provides an ANSI pseudoterminal that can be easily integrated into applications using [unsegen](https://crates.io/crates/unsegen).

## Getting Started

`unsegen_terminal` is [available on crates.io](https://crates.io/crates/unsegen_terminal). You can install it by adding this line to your `Cargo.toml`:

```toml
unsegen_terminal = "0.3.0"
```

## Examples

There is an example at the root of the crate [documentation](https://docs.rs/unsegen_terminal) which should be sufficient to get you going.

For a fully fledged application using `unsegen_terminal`, you can have a look at [ugdb](https://github.com/ftilde/ugdb), which was developed alongside `unsegen` and the primary motivation for it.

## Some notes on the state

The current API for passing on bytes from the pty to the terminal widget is a bit rough, but on the flip side is quite flexible and not tied to a specific event loop.
In the future, support for specific event loops (especially using Futures once they are stable) could be added.

Moreover, there are still a few unimplemented OSC handlers (see `terminalwindow.rs`), but the functionality is quite usable already.
Most notably, [ugdb](https://github.com/ftilde/ugdb), which uses `unsegen_terminal` itself, can debug itself.
Feel free to contribute missing functionality or to create issues if you hit a roadblock.

## Licensing

The majority of `unsegen_terminal` is released under the MIT license. This applies to all files that do not explicitly state to be licensed differently.
