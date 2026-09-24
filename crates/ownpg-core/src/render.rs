use std::fmt::Write as _;

use crate::classify::{self, Classification};
use crate::error::{Error, Result};

pub const IDENT_MAX_BYTES: usize = 63;

pub fn validate_ident(argument: &str, name: &str) -> Result<()> {
    let invalid = |detail: &str| Error::ArgumentInvalid {
        argument: argument.to_owned(),
        detail: format!("`{name}` {detail}"),
    };
    if name.is_empty() {
        return Err(invalid("is empty"));
    }
    if name.len() > IDENT_MAX_BYTES {
        return Err(invalid("is longer than 63 bytes"));
    }
    if name.contains('\0') {
        return Err(invalid("holds a NUL byte"));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid("holds a control character"));
    }
    if name.trim() != name {
        return Err(invalid("starts or ends with whitespace"));
    }
    Ok(())
}

#[must_use]
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[must_use]
pub fn quote_literal(text: &str) -> String {
    if text.contains('\\') {
        format!("E'{}'", text.replace('\\', "\\\\").replace('\'', "''"))
    } else {
        format!("'{}'", text.replace('\'', "''"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedName {
    pub schema: String,
    pub name: String,
}

impl QualifiedName {
    pub fn parse(argument: &str, input: &str, scoped_schema: &str) -> Result<Self> {
        let (schema, name) = split_name(input, scoped_schema);
        validate_ident(argument, &name)?;
        validate_ident("schema", &schema)?;
        if schema != scoped_schema && !classify::CATALOG_SCHEMAS.contains(&schema.as_str()) {
            return Err(Error::StatementRefused {
                rule: format!(
                    "object outside scoped schema: `{schema}.{name}` (this server serves `{scoped_schema}`)"
                ),
                mode: "any".to_owned(),
            });
        }
        Ok(Self { schema, name })
    }

    #[must_use]
    pub fn sql(&self) -> String {
        format!("{}.{}", quote_ident(&self.schema), quote_ident(&self.name))
    }

    #[must_use]
    pub fn display(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }
}

pub fn ident_list(argument: &str, names: &[String]) -> Result<String> {
    if names.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: "at least one name is required".to_owned(),
        });
    }
    let mut out = String::new();
    for (index, name) in names.iter().enumerate() {
        validate_ident(argument, name)?;
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&quote_ident(name));
    }
    Ok(out)
}

pub fn type_name(argument: &str, text: &str) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: "a type name is required".to_owned(),
        });
    }
    let probe = format!("SELECT NULL::{trimmed}");
    let parsed = classify::classify(&probe).map_err(|error| Error::ArgumentInvalid {
        argument: argument.to_owned(),
        detail: format!("`{trimmed}` is not a type name: {error}"),
    })?;
    if parsed.kind != "SelectStmt" || !parsed.relations.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: format!("`{trimmed}` is not a plain type name"),
        });
    }
    if let Some(rule) = parsed.refusals.first() {
        return Err(Error::StatementRefused {
            rule: rule.clone(),
            mode: "any".to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

pub fn operator(argument: &str, text: &str) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: "an operator is required".to_owned(),
        });
    }
    let probe = format!("SELECT NULL {trimmed} NULL");
    let parsed = classify::classify(&probe).map_err(|error| Error::ArgumentInvalid {
        argument: argument.to_owned(),
        detail: format!("`{trimmed}` is not an operator: {error}"),
    })?;
    if parsed.kind != "SelectStmt" || !parsed.relations.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: format!("`{trimmed}` is not a plain operator"),
        });
    }
    if let Some(rule) = parsed.refusals.first() {
        return Err(Error::StatementRefused {
            rule: rule.clone(),
            mode: "any".to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

pub fn partition_bound(argument: &str, text: &str) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: "a partition bound is required".to_owned(),
        });
    }
    let probe =
        format!("ALTER TABLE ownpg_probe_parent ATTACH PARTITION ownpg_probe_child {trimmed}");
    let parsed = classify::classify(&probe).map_err(|error| Error::ArgumentInvalid {
        argument: argument.to_owned(),
        detail: format!("`{trimmed}` is not a partition bound: {error}"),
    })?;
    if parsed.kind != "AlterTableStmt" {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: format!("`{trimmed}` is not a plain partition bound"),
        });
    }
    let unexpected = parsed.relations.iter().any(|relation| {
        relation.name != "ownpg_probe_parent" && relation.name != "ownpg_probe_child"
    });
    if unexpected {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: format!("`{trimmed}` names an object; a partition bound may not"),
        });
    }
    if let Some(rule) = parsed.refusals.first() {
        return Err(Error::StatementRefused {
            rule: rule.clone(),
            mode: "any".to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

pub fn returns_type(argument: &str, text: &str) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: "a return type is required".to_owned(),
        });
    }
    let probe = format!(
        "CREATE FUNCTION ownpg_probe_fn() RETURNS {trimmed} LANGUAGE sql AS $$ SELECT 1 $$"
    );
    let parsed = classify::classify(&probe).map_err(|error| Error::ArgumentInvalid {
        argument: argument.to_owned(),
        detail: format!("`{trimmed}` is not a return type: {error}"),
    })?;
    if parsed.kind != "CreateFunctionStmt" {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: format!("`{trimmed}` is not a plain return type"),
        });
    }
    if let Some(rule) = parsed.refusals.first() {
        return Err(Error::StatementRefused {
            rule: rule.clone(),
            mode: "any".to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

pub fn expression(argument: &str, text: &str) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: "an expression is required".to_owned(),
        });
    }
    let probe = format!("SELECT ({trimmed})");
    let parsed = classify::classify(&probe).map_err(|error| Error::ArgumentInvalid {
        argument: argument.to_owned(),
        detail: format!("`{trimmed}` is not a single expression: {error}"),
    })?;
    let single = matches!(
        (
            classify::select_shape(&probe),
            classify::select_shape("SELECT (1)"),
        ),
        (Some((1, shape)), Some((1, reference))) if shape == reference
    );
    if parsed.kind != "SelectStmt" || !single {
        return Err(Error::ArgumentInvalid {
            argument: argument.to_owned(),
            detail: format!(
                "`{trimmed}` is not a single expression; FROM, UNION, GROUP BY, ORDER BY, LIMIT, and extra columns are not allowed"
            ),
        });
    }
    if let Some(rule) = parsed.refusals.first() {
        return Err(Error::StatementRefused {
            rule: rule.clone(),
            mode: "any".to_owned(),
        });
    }
    Ok(trimmed.to_owned())
}

fn redacted(normalized: &str) -> String {
    if normalized.trim().is_empty() {
        return "(withheld)".to_owned();
    }
    crate::audit::short_statement(normalized).unwrap_or_else(|| {
        crate::shape::cut_graphemes(normalized, crate::audit::SHORT_STATEMENT_CAP)
    })
}

pub fn verify(sql: &str, expected_kinds: &[&str]) -> Result<Classification> {
    let classification = classify::classify(sql).map_err(|error| Error::ProtocolFailed {
        detail: format!(
            "the rendered statement did not parse: {error}; statement: {}",
            redacted(&pg_query::normalize(sql).unwrap_or_default())
        ),
    })?;
    if !expected_kinds.is_empty() && !expected_kinds.contains(&classification.kind.as_str()) {
        return Err(Error::ProtocolFailed {
            detail: format!(
                "the rendered statement is a {} where {} was expected; statement: {}",
                classification.kind,
                expected_kinds.join(" or "),
                redacted(&classification.normalized)
            ),
        });
    }
    Ok(classification)
}

#[derive(Debug, Default)]
pub struct Statement {
    text: String,
}

impl Statement {
    #[must_use]
    pub fn new(head: &str) -> Self {
        Self {
            text: head.to_owned(),
        }
    }

    pub fn push(&mut self, part: &str) -> &mut Self {
        if !self.text.is_empty() && !self.text.ends_with(' ') && !part.starts_with(' ') {
            self.text.push(' ');
        }
        self.text.push_str(part);
        self
    }

    pub fn push_if(&mut self, condition: bool, part: &str) -> &mut Self {
        if condition {
            self.push(part);
        }
        self
    }

    pub fn push_fmt(&mut self, args: std::fmt::Arguments<'_>) -> &mut Self {
        let mut part = String::new();
        let _ = part.write_fmt(args);
        self.push(&part)
    }

    #[must_use]
    pub fn finish(self) -> String {
        self.text
    }
}

#[must_use]
pub fn split_name(name: &str, scoped: &str) -> (String, String) {
    let trimmed = name.trim();
    let unquote = |part: &str| -> String {
        let part = part.trim();
        if part.len() >= 2 && part.starts_with('"') && part.ends_with('"') {
            part.get(1..part.len() - 1)
                .unwrap_or(part)
                .replace("\"\"", "\"")
        } else {
            part.to_ascii_lowercase()
        }
    };
    if let Some((schema, bare)) = split_qualified(trimmed) {
        (unquote(schema), unquote(bare))
    } else {
        (scoped.to_owned(), unquote(trimmed))
    }
}

fn split_qualified(name: &str) -> Option<(&str, &str)> {
    let mut in_quotes = false;
    for (index, c) in name.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            '.' if !in_quotes => return Some((name.get(..index)?, name.get(index + 1..)?)),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_split_on_the_unquoted_dot_and_fold_case_outside_quotes() {
        assert_eq!(
            split_name("orders", "app"),
            ("app".to_owned(), "orders".to_owned())
        );
        assert_eq!(
            split_name("Other.Orders", "app"),
            ("other".to_owned(), "orders".to_owned())
        );
        assert_eq!(
            split_name("\"Mixed.Case\".\"T\"", "app"),
            ("Mixed.Case".to_owned(), "T".to_owned())
        );
        assert_eq!(
            split_name("\"a\"\"b\"", "app"),
            ("app".to_owned(), "a\"b".to_owned())
        );
    }

    #[test]
    fn identifiers_are_validated_and_quoted() {
        validate_ident("table", "orders").unwrap();
        validate_ident("table", "Mixed Case").unwrap();
        assert!(validate_ident("table", "").is_err());
        assert!(validate_ident("table", &"x".repeat(64)).is_err());
        assert!(validate_ident("table", "bad\0name").is_err());
        assert!(validate_ident("table", " padded").is_err());
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
        assert_eq!(quote_literal("it's"), "'it''s'");
        assert_eq!(quote_literal("back\\slash"), "E'back\\\\slash'");
    }

    #[test]
    fn qualified_names_stay_inside_the_scoped_schema() {
        let name = QualifiedName::parse("table", "orders", "app").unwrap();
        assert_eq!(name.sql(), "\"app\".\"orders\"");
        let explicit = QualifiedName::parse("table", "app.\"Orders\"", "app").unwrap();
        assert_eq!(explicit.name, "Orders");
        let outside = QualifiedName::parse("table", "other.orders", "app").unwrap_err();
        assert!(outside.to_string().contains("outside scoped schema"));
        let catalog = QualifiedName::parse("table", "pg_catalog.pg_class", "app").unwrap();
        assert_eq!(catalog.schema, "pg_catalog");
    }

    #[test]
    fn type_names_and_expressions_must_parse_as_one_fragment() {
        assert_eq!(
            type_name("type", " numeric(10, 2) ").unwrap(),
            "numeric(10, 2)"
        );
        assert_eq!(type_name("type", "text[]").unwrap(), "text[]");
        assert!(type_name("type", "int; DROP TABLE x").is_err());
        assert!(type_name("type", "").is_err());
        assert_eq!(expression("default", "now()").unwrap(), "now()");
        assert!(expression("default", "1); DROP TABLE x; --").is_err());
        assert!(expression("default", "pg_read_file('/etc/passwd')").is_err());
        for good in [
            "id = 1",
            "status IN ('a', 'b') AND created_at > now() - interval '1 day'",
            "EXISTS (SELECT 1 FROM app.items i WHERE i.order_id = orders.id)",
            "(a + b) * 2",
        ] {
            expression("filter", good).unwrap_or_else(|error| panic!("{good}: {error}"));
        }
        for bad in [
            "true) UNION SELECT (1",
            "1) FROM orders WHERE (true",
            "1), (2",
            "true) ORDER BY (1",
            "true) LIMIT (0",
            "",
        ] {
            let error = expression("filter", bad).unwrap_err();
            assert_eq!(error.id().as_str(), "argument.invalid", "{bad}");
        }
    }

    #[test]
    fn a_type_name_that_smuggles_a_refused_function_is_turned_away() {
        let error = type_name("type", "int, pg_read_file('/etc/passwd')").unwrap_err();
        assert_eq!(error.id(), crate::error::ErrorId::StatementRefused);
        assert!(error.to_string().contains("pg_read_file"));
    }

    #[test]
    fn the_verify_guard_checks_the_statement_kind() {
        let parsed = verify("CREATE TABLE app.t (id int)", &["CreateStmt"]).unwrap();
        assert_eq!(parsed.class.as_str(), "ddl");
        assert!(verify("CREATE TABLE app.t (id int)", &["IndexStmt"]).is_err());
        assert!(verify("CREATE TABLE app.t (id int); DROP TABLE app.t", &[]).is_err());
    }

    #[test]
    fn a_refused_statement_never_echoes_a_password_literal_back_to_the_caller() {
        let wrong_kind = verify("CREATE ROLE app WITH PASSWORD 'hunter2'", &["IndexStmt"])
            .unwrap_err()
            .to_string();
        assert!(!wrong_kind.contains("hunter2"), "{wrong_kind}");
        assert!(wrong_kind.contains("PASSWORD $1"), "{wrong_kind}");
        let unparsable = verify("CREATE ROLE app WITH PASSWORD 'hunter2' MAYBE (", &[])
            .unwrap_err()
            .to_string();
        assert!(!unparsable.contains("hunter2"), "{unparsable}");
        assert!(unparsable.contains("(withheld)"), "{unparsable}");
    }

    #[test]
    fn the_statement_builder_joins_parts_with_single_spaces() {
        let mut statement = Statement::new("ALTER TABLE");
        statement
            .push("\"app\".\"t\"")
            .push_if(false, "NEVER")
            .push_if(true, "ADD COLUMN");
        statement.push_fmt(format_args!("{} {}", quote_ident("c"), "int"));
        assert_eq!(
            statement.finish(),
            "ALTER TABLE \"app\".\"t\" ADD COLUMN \"c\" int"
        );
    }
}
