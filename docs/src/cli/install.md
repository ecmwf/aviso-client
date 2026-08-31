# Install the CLI

The fastest way to get `aviso` is pip. The Python package bundles the CLI, so
one install works even if you never write a line of Python.

## From PyPI

```bash
pip install pyaviso
aviso --version
```

The wheel installs a console script that runs the Rust CLI in-process through
the extension, so you do not need a Rust toolchain. See the
[Python install page](../python/install.md) for wheel coverage and details.

## From crates.io

If you have a Rust toolchain and prefer a native binary:

```bash
cargo install aviso-cli
aviso --version
```

`cargo install` builds from source and puts the binary in `~/.cargo/bin/aviso`.
That directory is on your PATH when you installed Rust through
[rustup](https://rustup.rs/). Both install paths give you the same `aviso`
command and behaviour.

## From a git checkout

For an unreleased version, or when contributing:

```bash
git clone https://github.com/ecmwf/aviso-client.git
cd aviso-client
cargo install --path crates/aviso-cli
```

## Verify

```bash
aviso --version
aviso --help
```

You should see the subcommands listed.

## Where the binary lives

The binary is called `aviso`, not `aviso-cli`. The crate name carries the `-cli`
suffix to leave the unprefixed `aviso` for the library, but the installed
executable is plain `aviso` so it reads cleanly on the command line.

```bash
# Linux, macOS
which aviso
# pip install: <venv-or-user-base>/bin/aviso
# cargo install: /home/you/.cargo/bin/aviso
```

## Shell completions

```bash
# Bash
aviso completions bash > ~/.local/share/bash-completion/completions/aviso

# Zsh
aviso completions zsh > ~/.local/share/zsh/site-functions/_aviso

# Fish
aviso completions fish > ~/.config/fish/completions/aviso.fish

# Elvish
aviso completions elvish > ~/.config/elvish/lib/aviso.elv
```

Restart the shell (or source the file) for completions to take effect.

## Upgrading

- pip: `pip install --upgrade pyaviso`.
- cargo: `cargo install aviso-cli` again. Cargo replaces the binary in place;
  add `--force` to rebuild from scratch.

## Uninstalling

```bash
pip uninstall pyaviso        # pip install
cargo uninstall aviso-cli    # cargo install
```

A cargo-installed binary can also be deleted from `~/.cargo/bin/aviso` by hand.
Configuration and state files in `~/.config/aviso/` are left alone; remove them
if you want a clean slate:

```bash
rm -rf ~/.config/aviso
```

## What next

- [Quickstart](./quickstart.md): your first run.
- [Configuration](./configuration.md): config files, environment variables, TLS
  settings.
