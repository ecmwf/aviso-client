//! `aviso completions <SHELL>` subcommand.
//!
//! Prints the shell-completion script for the requested shell to
//! stdout. Operators redirect the output to a shell-specific path
//! (the docs/src/usage/cli.md 'completions' section names the
//! conventional path for each shell). The script is generated from
//! the clap derive Cli definition via [`clap_complete::generate`],
//! so it stays in sync with the real flag and subcommand surface
//! automatically.
//!
//! The program name passed to `generate` is hard-coded to `"aviso"`
//! (the binary name from `[[bin]] name = "aviso"`), NOT the crate
//! name `aviso-cli`; without the explicit binding `clap_complete`
//! would emit a script that names `aviso-cli` and the operator's
//! shell would refuse to use it.

use anyhow::Result;
use clap::CommandFactory;

/// Runs the `aviso completions <SHELL>` subcommand.
pub(crate) fn run(shell: clap_complete::Shell) -> Result<()> {
    let mut cmd = crate::Cli::command();
    let mut buf: Vec<u8> = Vec::new();
    clap_complete::generate(shell, &mut cmd, "aviso", &mut buf);
    crate::output::write_stdout_bytes(&buf)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on completions generation is the expected diagnostic"
)]
mod tests {
    use clap::CommandFactory;
    use clap_complete::Shell;

    #[test]
    fn generates_non_empty_bash_completions() {
        let mut cmd = crate::Cli::command();
        let mut buf: Vec<u8> = Vec::new();
        clap_complete::generate(Shell::Bash, &mut cmd, "aviso", &mut buf);
        assert!(!buf.is_empty());
        let s = String::from_utf8_lossy(&buf);
        assert!(
            s.contains("aviso") || s.contains("_aviso"),
            "completion script should name the binary"
        );
    }

    #[test]
    fn generates_non_empty_zsh_completions() {
        let mut cmd = crate::Cli::command();
        let mut buf: Vec<u8> = Vec::new();
        clap_complete::generate(Shell::Zsh, &mut cmd, "aviso", &mut buf);
        assert!(!buf.is_empty());
    }

    #[test]
    fn generates_non_empty_fish_completions() {
        let mut cmd = crate::Cli::command();
        let mut buf: Vec<u8> = Vec::new();
        clap_complete::generate(Shell::Fish, &mut cmd, "aviso", &mut buf);
        assert!(!buf.is_empty());
    }
}
