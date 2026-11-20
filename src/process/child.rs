//! A started child and the streams opened for it. Win32 children with an explicit handle
//! list cannot be constructed as `std::process::Child`, but expose the same owned operations.
use std::io::{self, Read, Write};
use std::process::{self, ExitStatus};

pub struct Child {
    process: Handle,
    pub stdin: Option<Box<dyn Write + Send>>,
    pub stdout: Option<Box<dyn Read + Send>>,
    pub stderr: Option<Box<dyn Read + Send>>,
}

enum Handle {
    Std(process::Child),
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
            Handle::Std(_) => None,
            Handle::Windows(child) => child.compiler_job(),
        }
    }

    pub fn id(&self) -> u32 {
        match &self.process {
            Handle::Std(child) => child.id(),
            #[cfg(windows)]
            Handle::Windows(child) => child.id(),
        }
    }

    pub fn kill(&mut self) -> io::Result<()> {
        match &mut self.process {
            Handle::Std(child) => child.kill(),
            #[cfg(windows)]
            Handle::Windows(child) => child.kill(),
        }
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        drop(self.stdin.take());
        match &mut self.process {
            Handle::Std(child) => child.wait(),
            #[cfg(windows)]
            Handle::Windows(child) => child.wait(),
        }
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match &mut self.process {
            Handle::Std(child) => child.try_wait(),
            #[cfg(windows)]
            Handle::Windows(child) => child.try_wait(),
        }
    }
}
