//! secure-folder-ctl: command line client of the Secure Folder session service.
//!
//! A thin front-end over `io.github.axel7083.SecureFolder.Vaults1`: it holds
//! no vault logic, so it doubles as a reference for other integrations.

use std::io::{IsTerminal, Read};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use sf_dbus::{Error, VaultInfo, Vaults1Proxy};
use zbus::{Connection, Fd};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(name = "secure-folder-ctl", version, about = "Manage Secure Folder vaults")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List vaults and their state
    List,
    /// Create a new vault
    Create {
        name: String,
        /// Read the passphrase from standard input instead of prompting
        #[arg(long)]
        passphrase_stdin: bool,
    },
    /// Unlock a vault (by name or id) with its passphrase
    Unlock {
        vault: String,
        #[arg(long, conflicts_with = "fingerprint")]
        passphrase_stdin: bool,
        /// Use fingerprint / login password (system dialog) instead
        #[arg(long)]
        fingerprint: bool,
    },
    /// Turn fingerprint unlock on (asks the vault passphrase, then the system)
    FingerprintOn {
        vault: String,
        #[arg(long)]
        passphrase_stdin: bool,
    },
    /// Turn fingerprint unlock off
    FingerprintOff { vault: String },
    /// Lock a vault (by name or id)
    Lock {
        vault: String,
        /// Lock even if files are open (unsaved changes in apps may be lost)
        #[arg(long)]
        force: bool,
    },
    /// Lock every vault
    LockAll,
    /// Forget a locked vault (its encrypted files are kept)
    Remove { vault: String },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("secure-folder-ctl: {}", e.message());
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), Error> {
    let conn = Connection::session().await?;
    let vaults = Vaults1Proxy::new(&conn).await?;
    match cli.command {
        Command::List => {
            for v in vaults.list().await? {
                let fp = match vaults.get_fingerprint_status(&v.id).await {
                    Ok(s) if s.enrolled => "fingerprint",
                    _ => "-",
                };
                println!("{}\t{}\t{}\t{}\t{}", v.id, v.state, fp, v.name, v.mount_point);
            }
        }
        Command::Create { name, passphrase_stdin } => {
            let pass = passphrase(passphrase_stdin, true)?;
            let id = vaults.create(&name, pass).await?;
            println!("{id}");
        }
        Command::Unlock { vault, passphrase_stdin, fingerprint } => {
            let v = resolve(&vaults, &vault).await?;
            let mount_point = if fingerprint {
                vaults.unlock_with_key_service(&v.id).await?
            } else {
                vaults.unlock_with_passphrase(&v.id, passphrase(passphrase_stdin, false)?).await?
            };
            println!("{mount_point}");
        }
        Command::FingerprintOn { vault, passphrase_stdin } => {
            let v = resolve(&vaults, &vault).await?;
            vaults.enroll(&v.id, passphrase(passphrase_stdin, false)?).await?;
        }
        Command::FingerprintOff { vault } => {
            let v = resolve(&vaults, &vault).await?;
            vaults.unenroll(&v.id).await?;
        }
        Command::Lock { vault, force } => {
            let v = resolve(&vaults, &vault).await?;
            vaults.lock(&v.id, force).await?;
        }
        Command::LockAll => vaults.lock_all().await?,
        Command::Remove { vault } => {
            let v = resolve(&vaults, &vault).await?;
            vaults.remove(&v.id).await?;
        }
    }
    Ok(())
}

/// Accepts an exact id, or a name matching exactly one vault.
async fn resolve(vaults: &Vaults1Proxy<'_>, query: &str) -> Result<VaultInfo, Error> {
    let list = vaults.list().await?;
    if let Some(v) = list.iter().find(|v| v.id == query) {
        return Ok(v.clone());
    }
    let mut matches = list.into_iter().filter(|v| v.name == query);
    match (matches.next(), matches.next()) {
        (Some(v), None) => Ok(v),
        (Some(_), Some(_)) => Err(Error::InvalidArgument(format!("several vaults are named {query:?}, use the id"))),
        (None, _) => Err(Error::UnknownVault(format!("no vault named {query:?}"))),
    }
}

/// Reads the passphrase (prompt or stdin) and returns the read end of a pipe
/// holding it: the secret travels as a file descriptor, never in the message.
fn passphrase(from_stdin: bool, confirm: bool) -> Result<Fd<'static>, Error> {
    let io = |e: std::io::Error| Error::Failed(e.to_string());
    let secret = if from_stdin || !std::io::stdin().is_terminal() {
        let mut s = Zeroizing::new(String::new());
        std::io::stdin().read_to_string(&mut s).map_err(io)?;
        if s.ends_with('\n') {
            s.pop();
        }
        s
    } else {
        let s = Zeroizing::new(rpassword::prompt_password("Passphrase: ").map_err(io)?);
        if confirm {
            let again = Zeroizing::new(rpassword::prompt_password("Repeat passphrase: ").map_err(io)?);
            if *again != *s {
                return Err(Error::InvalidArgument("passphrases do not match".into()));
            }
        }
        s
    };
    sf_dbus::secret_fd(secret.as_bytes()).map_err(io)
}
