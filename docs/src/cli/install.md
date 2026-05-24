# Install the CLI

The fastest way to get `aviso` is `cargo install`. You will need [Rust installed](https://rustup.rs/) first.

## From crates.io

```bash
cargo install aviso-cli
aviso --version
```

`cargo install` builds from source and puts the binary in `~/.cargo/bin/aviso`. That directory is on your PATH when you installed Rust through rustup.

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

The binary is called `aviso`, not `aviso-cli`. The crate name carries the `-cli` suffix to leave the unprefixed `aviso` for the library, but the installed executable is plain `aviso` so it reads cleanly on the command line.

```bash
which aviso
# /home/you/.cargo/bin/aviso
```

## Shell completions

```bash
# Bash
aviso completions bash > ~/.local/share/bash-completion/completions/aviso

# Zsh
aviso completions zsh > ~/.local/share/zsh/site-functions/_aviso

# Fish
aviso completions fish > ~/.config/fish/completions/aviso.fish

# PowerShell
aviso completions powershell >> $PROFILE
```

Restart the shell (or source the file) for completions to take effect.

## Upgrading

`cargo install aviso-cli` again. Cargo replaces the binary in place. To force a rebuild from scratch, add `--force`.

## Uninstalling

```bash
cargo uninstall aviso-cli
```

You can also delete `~/.cargo/bin/aviso` by hand. Configuration and state files in `~/.config/aviso/` are left alone; remove them if you want a clean slate:

```bash
rm -rf ~/.config/aviso
```

## What next

- [Quickstart](./quickstart.md): your first run.
- [Configuration](./configuration.md): config files, environment variables, TLS settings.
