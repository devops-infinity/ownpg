use crate::error::{Error, Result};

pub fn statement_count(sql: &str) -> Result<usize> {
    let statements =
        pg_query::split_with_parser(sql).map_err(|error| Error::StatementUnparsable {
            reason: error.to_string(),
        })?;
    Ok(statements.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ErrorId, ExitClass};

    #[test]
    fn one_statement_counts_as_one() {
        assert_eq!(statement_count("SELECT 1").unwrap(), 1);
        assert_eq!(statement_count("SELECT 1;").unwrap(), 1);
    }

    #[test]
    fn a_batch_is_counted_by_the_parser_and_not_by_semicolons() {
        assert_eq!(statement_count("SELECT 1; DROP TABLE t").unwrap(), 2);
        assert_eq!(statement_count("SELECT ';'; SELECT 2").unwrap(), 2);
        assert_eq!(statement_count("SELECT $$a;b$$").unwrap(), 1);
    }

    #[test]
    fn an_empty_input_holds_no_statement() {
        assert_eq!(statement_count("").unwrap(), 0);
        assert_eq!(statement_count("   \n").unwrap(), 0);
    }

    #[test]
    fn a_statement_the_parser_refuses_is_reported_with_its_reason() {
        let error = statement_count("SELECT FROM WHERE").unwrap_err();
        assert_eq!(error.id(), ErrorId::StatementUnparsable);
        assert_eq!(error.exit_class(), ExitClass::Refused);
        assert!(error.to_string().contains("syntax error"), "{error}");
    }
}
