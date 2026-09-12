use ownpg_core::{Error, ExitClass, Result};

use crate::output::{emit, stdout_error};

pub(crate) fn verify(path: &std::path::Path) -> Result<ExitClass> {
    match ownpg_core::audit::verify_chain(path) {
        Ok(line_count) => {
            emit(|out| {
                writeln!(
                    out,
                    "{}: {line_count} chained lines verified",
                    path.display()
                )
            })
            .map_err(stdout_error)?;
            Ok(ExitClass::Success)
        }
        Err(problem) => Err(Error::AuditTampered {
            path: path.to_path_buf(),
            detail: problem,
        }),
    }
}
