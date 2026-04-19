# hmm-rs

Rust rewrite of `h-m-m`, a keyboard-centric terminal mind-map tool.

This implementation treats the map as a visual layer for idea shaping and issue tracking,
instead of a fully stateful task system by itself.

## Positioning

One gap in `hmm` is that a node is not inherently stateful.
It is great for structure, hierarchy, and thinking, but not for representing the full lifecycle
of tracked work on its own.

This Rust version embeds Linear into that workflow.
In practice, the tool serves as a visualized interface for idea planning and issue tracking:

- use the tree to shape ideas, scope, and breakdowns
- push selected nodes or subtrees into Linear issues
- keep the map as the planning surface and Linear as the execution system

That means `hmm-rs` is not trying to replace Linear.
It is meant to bridge free-form visual thinking with actual issue management.

## Screenshot

Current UI snapshot:

<p align="center">
  <img src="docs/Screenshot.png" alt="hmm-rs current UI" width="600" />
</p>

## Global install

The binary name is `hmm`, so the easiest global install is:

```sh
cargo install --path . --locked --force
```

This installs `hmm` into Cargo's bin directory, usually:

```sh
~/.cargo/bin
```

Make sure it is on your `PATH`:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

Then you can run:

```sh
hmm
hmm my-map.hmm
```

## Local helper install script

You can also use:

```sh
./scripts/install-global.sh
```

That installs the current repo as a global `hmm` command using Cargo.

## External dependencies

`hmm-rs` depends on a few external tools depending on which features you use:

- `cargo` / Rust toolchain for building and installing
- `linear` CLI for Linear issue create/update/open flows
- standard platform opener tools such as `open` on macOS or `xdg-open` on Linux

If you use only local editing, the Linear dependency is optional.
If you use issue push/open workflows, `linear` CLI must be installed and authenticated.

## Usage

```sh
hmm
hmm path/to/file.hmm
```

If no file is provided, `hmm` opens `./mind.hmm` in the current directory.
If `./mind.hmm` does not exist yet, it starts a new map that will save to that path.
