use std::io::{self, Write};

use ownpg_core::Error;

pub(crate) fn emit<F>(render: F) -> io::Result<()>
where
    F: FnOnce(&mut dyn Write) -> io::Result<()>,
{
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    match render(&mut handle).and_then(|()| handle.flush()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn stdout_error(source: io::Error) -> Error {
    Error::OutputUnwritable {
        target: "stdout".to_owned(),
        source,
    }
}

pub(crate) fn report_error(error: &Error) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "error: {error}");
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        let _ = writeln!(stderr, "  caused by: {cause}");
        source = cause.source();
    }
    let remedy = error.remedy();
    if !remedy.is_empty() {
        let _ = writeln!(stderr, "  try: {remedy}");
    }
    let _ = writeln!(stderr, "  code: {}", error.id());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_pipe_ends_the_write_quietly() {
        let outcome = emit(|_: &mut dyn Write| Err(io::Error::from(io::ErrorKind::BrokenPipe)));
        assert!(outcome.is_ok());
    }

    #[test]
    fn any_other_write_failure_is_reported() {
        let outcome =
            emit(|_: &mut dyn Write| Err(io::Error::from(io::ErrorKind::PermissionDenied)));
        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn a_stdout_failure_carries_the_output_error_id() {
        let error = stdout_error(io::Error::from(io::ErrorKind::PermissionDenied));
        assert_eq!(error.id().as_str(), "output.unwritable");
    }
}
