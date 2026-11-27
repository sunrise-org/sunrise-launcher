//! A started child and the streams opened for it. Win32 children with an explicit handle
//! list cannot be constructed as `std::process::Child`, but expose the same owned operations.
use std::io::{self, Read, Write};
use std::process::{self, ExitStatus};

pub fn unconfirmed_exit(error: io::Error) -> String {
    crate::i18n::trf(
        "Sunrise started, but the launcher couldn't confirm that it stopped. Close the game if it's still open.\n{reason}",
        &[("reason", &error.to_string())],
    )
}

pub struct Child {
    process: Handle,
    pub stdin: Option<Box<dyn Write + Send>>,
    pub stdout: Option<Box<dyn Read + Send>>,
    pub stderr: Option<Box<dyn Read + Send>>,
}

enum Handle {
    Std(process::Child),
    Relayed(super::relay_lifetime::RelayedGame),
    #[cfg(windows)]
    Windows(super::leases::WindowsChild),
}

impl From<process::Child> for Child {
    fn from(mut child: process::Child) -> Self {
        Self {
            stdin: child.stdin.take().map(|io| Box::new(io) as _),
            stdout: child.stdout.take().map(|io| Box::new(io) as _),
            stderr: child.stderr.take().map(|io| Box::new(io) as _),
            process: Handle::Std(child),
        }
    }
}

impl Child {
    pub(super) fn relayed(game: super::relay_lifetime::RelayedGame, wrapper: &mut Self) -> Self {
        Self {
            process: Handle::Relayed(game),
            stdin: wrapper.stdin.take(),
            stdout: wrapper.stdout.take(),
            stderr: wrapper.stderr.take(),
        }
    }

    pub(crate) fn is_relayed(&self) -> bool {
        matches!(self.process, Handle::Relayed(_))
    }

    #[cfg(windows)]
    pub(super) fn windows(
        child: super::leases::WindowsChild,
        pipes: Option<(io::PipeWriter, io::PipeReader, io::PipeReader)>,
    ) -> Self {
        let (stdin, stdout, stderr) = if let Some((input, output, error)) = pipes {
            (
                Some(Box::new(input) as _),
                Some(Box::new(output) as _),
                Some(Box::new(error) as _),
            )
        } else {
            (None, None, None)
        };
        Self {
            process: Handle::Windows(child),
            stdin,
            stdout,
            stderr,
        }
    }

    #[cfg(windows)]
    pub(crate) fn compiler_job(&self) -> Option<std::sync::Arc<super::leases::CompilerJob>> {
        match &self.process {
            Handle::Std(_) | Handle::Relayed(_) => None,
            Handle::Windows(child) => child.compiler_job(),
        }
    }

    #[cfg(windows)]
    pub(crate) fn preparer_job(&self) -> Option<std::sync::Arc<super::leases::CompilerJob>> {
        match &self.process {
            Handle::Std(_) | Handle::Relayed(_) => None,
            Handle::Windows(child) => child.preparer_job(),
        }
    }

    #[cfg(windows)]
    pub(super) fn wrapper_job(&self) -> Option<std::sync::Arc<super::leases::WrapperJob>> {
        match &self.process {
            Handle::Std(_) | Handle::Relayed(_) => None,
            Handle::Windows(child) => child.wrapper_job(),
        }
    }

    #[cfg(windows)]
    pub(super) fn wrapper_leader_exited(&self) -> io::Result<bool> {
        match &self.process {
            Handle::Std(_) | Handle::Relayed(_) => Err(io::ErrorKind::InvalidInput.into()),
            Handle::Windows(child) => child.wrapper_leader_exited(),
        }
    }

    pub fn id(&self) -> u32 {
        match &self.process {
            Handle::Std(child) => child.id(),
            Handle::Relayed(child) => child.id(),
            #[cfg(windows)]
            Handle::Windows(child) => child.id(),
        }
    }

    pub fn kill(&mut self) -> io::Result<()> {
        match &mut self.process {
            Handle::Std(child) => child.kill(),
            Handle::Relayed(child) => child.kill(),
            #[cfg(windows)]
            Handle::Windows(child) => child.kill(),
        }
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        drop(self.stdin.take());
        match &mut self.process {
            Handle::Std(child) => child.wait(),
            Handle::Relayed(child) => child.wait(),
            #[cfg(windows)]
            Handle::Windows(child) => child.wait(),
        }
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match &mut self.process {
            Handle::Std(child) => child.try_wait(),
            Handle::Relayed(child) => child.try_wait(),
            #[cfg(windows)]
            Handle::Windows(child) => child.try_wait(),
        }
    }
}
