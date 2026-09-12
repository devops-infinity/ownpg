use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum ExitClass {
    Success = 0,
    Runtime = 1,
    Usage = 2,
    Refused = 4,
    External = 5,
    Interrupted = 130,
}

impl ExitClass {
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

impl fmt::Display for ExitClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorId {
    CommandUnknown,
    OutputUnwritable,
    StatementUnparsable,
}

impl ErrorId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommandUnknown => "command.unknown",
            Self::OutputUnwritable => "output.unwritable",
            Self::StatementUnparsable => "statement.unparsable",
        }
    }
}

impl fmt::Display for ErrorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("`{name}` is not a command")]
    CommandUnknown { name: String, known: Vec<String> },

    #[error("output could not be written to {target}")]
    OutputUnwritable {
        target: String,
        #[source]
        source: std::io::Error,
    },

    #[error("the statement could not be parsed: {reason}")]
    StatementUnparsable { reason: String },
}

impl Error {
    #[must_use]
    pub const fn id(&self) -> ErrorId {
        match self {
            Self::CommandUnknown { .. } => ErrorId::CommandUnknown,
            Self::OutputUnwritable { .. } => ErrorId::OutputUnwritable,
            Self::StatementUnparsable { .. } => ErrorId::StatementUnparsable,
        }
    }

    #[must_use]
    pub const fn exit_class(&self) -> ExitClass {
        match self {
            Self::CommandUnknown { .. } => ExitClass::Usage,
            Self::OutputUnwritable { .. } => ExitClass::Runtime,
            Self::StatementUnparsable { .. } => ExitClass::Refused,
        }
    }

    #[must_use]
    pub fn remedy(&self) -> String {
        match self {
            Self::CommandUnknown { known, .. } => {
                if known.is_empty() {
                    "Run `ownpg --help` to see every command.".to_owned()
                } else {
                    format!("Use one of: {}.", known.join(", "))
                }
            }
            Self::OutputUnwritable { .. } => {
                "Check that the target is writable and has free space.".to_owned()
            }
            Self::StatementUnparsable { .. } => {
                "Check the statement against the PostgreSQL manual, and send one statement per call."
                    .to_owned()
            }
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    fn every_error() -> Vec<Error> {
        vec![
            Error::CommandUnknown {
                name: "nope".to_owned(),
                known: vec!["man".to_owned(), "completions".to_owned()],
            },
            Error::OutputUnwritable {
                target: "stdout".to_owned(),
                source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no"),
            },
            Error::StatementUnparsable {
                reason: "syntax error at or near \"FROM\"".to_owned(),
            },
        ]
    }

    #[test]
    fn exit_codes_are_the_documented_contract() {
        assert_eq!(ExitClass::Success.code(), 0);
        assert_eq!(ExitClass::Runtime.code(), 1);
        assert_eq!(ExitClass::Usage.code(), 2);
        assert_eq!(ExitClass::Refused.code(), 4);
        assert_eq!(ExitClass::External.code(), 5);
        assert_eq!(ExitClass::Interrupted.code(), 130);
        assert_eq!(
            [
                ExitClass::Success,
                ExitClass::Runtime,
                ExitClass::Usage,
                ExitClass::Refused,
                ExitClass::External,
                ExitClass::Interrupted,
            ]
            .len(),
            6,
            "the documented contract is exactly these six codes"
        );
    }

    #[test]
    fn every_exit_class_displays_as_its_code() {
        assert_eq!(ExitClass::Refused.to_string(), "4");
        assert_eq!(ExitClass::Interrupted.to_string(), "130");
    }

    #[test]
    fn every_error_has_an_id_and_a_remedy() {
        for case in every_error() {
            assert!(!case.id().as_str().is_empty(), "{case:?}");
            assert!(!case.remedy().is_empty(), "{case:?}");
            assert_eq!(case.id().to_string(), case.id().as_str());
        }
    }

    #[test]
    fn every_error_maps_to_the_documented_exit_class() {
        let classes: Vec<ExitClass> = every_error().iter().map(Error::exit_class).collect();
        assert_eq!(
            classes,
            vec![ExitClass::Usage, ExitClass::Runtime, ExitClass::Refused]
        );
    }

    #[test]
    fn error_id_strings_are_the_stable_contract() {
        assert_eq!(ErrorId::CommandUnknown.as_str(), "command.unknown");
        assert_eq!(ErrorId::OutputUnwritable.as_str(), "output.unwritable");
        assert_eq!(
            ErrorId::StatementUnparsable.as_str(),
            "statement.unparsable"
        );
    }

    #[test]
    fn an_unknown_command_names_the_commands_that_exist() {
        let error = Error::CommandUnknown {
            name: "mna".to_owned(),
            known: vec!["man".to_owned()],
        };
        assert_eq!(error.remedy(), "Use one of: man.");
        assert_eq!(error.to_string(), "`mna` is not a command");
    }

    #[test]
    fn an_unwritable_output_keeps_its_cause() {
        let error = Error::OutputUnwritable {
            target: "stdout".to_owned(),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no"),
        };
        assert!(std::error::Error::source(&error).is_some());
    }
}
