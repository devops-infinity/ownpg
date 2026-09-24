use crate::classify::{CascadeTarget, Classification};
use crate::engine::Engine;
use crate::error::Result;
use crate::render::{quote_ident, quote_literal};

use super::catalog::read_column;

pub const LISTED_DEPENDENTS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependent {
    pub kind: String,
    pub identity: String,
    pub schema: Option<String>,
}

impl Dependent {
    #[must_use]
    pub fn label(&self) -> String {
        format!("{} {}", self.kind, self.identity)
    }
}

fn text_array(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| quote_literal(item)).collect();
    format!("ARRAY[{}]::text[]", quoted.join(", "))
}

fn address_sql(target: &CascadeTarget) -> String {
    match (&target.args, target.kind) {
        (None, "function" | "procedure" | "routine" | "aggregate") => {
            let qualified: Vec<String> =
                target.names.iter().map(|part| quote_ident(part)).collect();
            format!(
                "SELECT 'pg_catalog.pg_proc'::pg_catalog.regclass::oid, pg_catalog.to_regproc({})::oid, 0::int4",
                quote_literal(&qualified.join("."))
            )
        }
        (args, kind) => format!(
            "SELECT classid, objid, objsubid FROM pg_catalog.pg_get_object_address({}, {}, {})",
            quote_literal(kind),
            text_array(&target.names),
            text_array(args.as_deref().unwrap_or_default())
        ),
    }
}

const WALK_SQL: &str = "walk(classid, objid, objsubid) AS ( \
        SELECT d.classid, d.objid, d.objsubid FROM pg_catalog.pg_depend d JOIN target t \
          ON d.refclassid = t.classid AND d.refobjid = t.objid AND (t.objsubid = 0 OR d.refobjsubid = t.objsubid) \
         WHERE d.deptype = 'n' \
        UNION \
        SELECT CASE WHEN d.classid = w.classid AND d.objid = w.objid THEN d.refclassid ELSE d.classid END, \
               CASE WHEN d.classid = w.classid AND d.objid = w.objid THEN d.refobjid ELSE d.objid END, \
               CASE WHEN d.classid = w.classid AND d.objid = w.objid THEN 0 ELSE d.objsubid END \
          FROM walk w JOIN pg_catalog.pg_depend d \
            ON (d.refclassid = w.classid AND d.refobjid = w.objid AND (w.objsubid = 0 OR d.refobjsubid = w.objsubid) AND d.deptype IN ('n', 'a', 'i')) \
            OR (d.classid = w.classid AND d.objid = w.objid AND d.deptype = 'i') \
    ) \
    SELECT DISTINCT o.type::text, o.identity::text, \
           COALESCE(o.schema, (SELECT n.nspname FROM pg_catalog.pg_depend p \
                JOIN pg_catalog.pg_class c ON p.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass AND c.oid = p.refobjid \
                JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
               WHERE p.classid = w.classid AND p.objid = w.objid AND p.deptype IN ('a', 'i') LIMIT 1))::text \
      FROM walk w CROSS JOIN LATERAL pg_catalog.pg_identify_object(w.classid, w.objid, w.objsubid) o \
     WHERE NOT EXISTS (SELECT 1 FROM target t WHERE t.classid = w.classid AND t.objid = w.objid) \
       AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend i WHERE i.classid = w.classid AND i.objid = w.objid AND i.deptype = 'i') \
     ORDER BY 3 NULLS FIRST, 1, 2";

pub async fn dependents(
    engine: &Engine,
    classification: &Classification,
) -> Result<Vec<Dependent>> {
    let mut addresses = Vec::new();
    for target in &classification.cascade_targets {
        let rows = match engine.catalog_rows(&address_sql(target), &[]).await {
            Ok(rows) => rows,
            Err(_) if target.missing_ok => continue,
            Err(error) => return Err(error),
        };
        for row in rows {
            let classid: Option<u32> = read_column(&row, 0)?;
            let objid: Option<u32> = read_column(&row, 1)?;
            let objsubid: i32 = read_column(&row, 2)?;
            if let (Some(classid), Some(objid)) = (classid, objid) {
                addresses.push(format!("({classid}::oid, {objid}::oid, {objsubid})"));
            }
        }
    }
    if addresses.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "WITH RECURSIVE target(classid, objid, objsubid) AS (VALUES {}), {WALK_SQL}",
        addresses.join(", ")
    );
    let rows = engine.catalog_rows(&sql, &[]).await?;
    rows.iter()
        .map(|row| {
            Ok(Dependent {
                kind: read_column(row, 0)?,
                identity: read_column(row, 1)?,
                schema: read_column(row, 2)?,
            })
        })
        .collect()
}

#[must_use]
pub fn listed(dependents: &[Dependent]) -> Vec<String> {
    let mut labels: Vec<String> = dependents
        .iter()
        .take(LISTED_DEPENDENTS)
        .map(Dependent::label)
        .collect();
    if dependents.len() > LISTED_DEPENDENTS {
        labels.push(format!("and {} more", dependents.len() - LISTED_DEPENDENTS));
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_becomes_an_object_address_lookup() {
        let table = CascadeTarget {
            kind: "table",
            names: vec!["app".to_owned(), "o'rders".to_owned()],
            args: None,
            missing_ok: false,
        };
        assert_eq!(
            address_sql(&table),
            "SELECT classid, objid, objsubid FROM pg_catalog.pg_get_object_address('table', ARRAY['app', 'o''rders']::text[], ARRAY[]::text[])"
        );
        let routine = CascadeTarget {
            kind: "function",
            names: vec!["app".to_owned(), "Touch".to_owned()],
            args: None,
            missing_ok: false,
        };
        assert!(
            address_sql(&routine).contains("to_regproc('\"app\".\"Touch\"')"),
            "{}",
            address_sql(&routine)
        );
    }

    #[test]
    fn a_long_list_is_cut_with_a_count() {
        let many: Vec<Dependent> = (0..LISTED_DEPENDENTS + 3)
            .map(|index| Dependent {
                kind: "view".to_owned(),
                identity: format!("app.v{index}"),
                schema: Some("app".to_owned()),
            })
            .collect();
        let labels = listed(&many);
        assert_eq!(labels.len(), LISTED_DEPENDENTS + 1);
        assert_eq!(labels.last().map(String::as_str), Some("and 3 more"));
    }
}
