use tokio_postgres::Client;

use crate::error::{Error, Result};

const ROLE_QUERY: &str = "SELECT r.rolname::text, r.rolsuper, r.rolbypassrls, r.rolcreatedb, r.rolcreaterole, r.rolreplication, \
    CASE WHEN EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'rds_superuser') \
         THEN pg_catalog.pg_has_role(r.oid, 'rds_superuser', 'member') ELSE false END, \
    COALESCE((SELECT pg_catalog.array_agg(b.rolname::text ORDER BY b.rolname) FROM pg_catalog.pg_auth_members m \
              JOIN pg_catalog.pg_roles b ON b.oid = m.roleid WHERE m.member = r.oid), ARRAY[]::text[]) \
    FROM pg_catalog.pg_roles r WHERE r.rolname = current_user";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleProfile {
    pub name: String,
    pub superuser: bool,
    pub bypass_rls: bool,
    pub create_db: bool,
    pub create_role: bool,
    pub replication: bool,
    pub rds_superuser: bool,
    pub memberships: Vec<String>,
}

impl RoleProfile {
    pub async fn load(client: &Client) -> Result<Self> {
        let row = client
            .query_one(ROLE_QUERY, &[])
            .await
            .map_err(|error| super::describe_sqlstate(&error))?;
        let read = |index: usize| -> Result<bool> {
            row.try_get::<_, bool>(index)
                .map_err(|error| super::describe_sqlstate(&error))
        };
        Ok(Self {
            name: row
                .try_get::<_, String>(0)
                .map_err(|error| super::describe_sqlstate(&error))?,
            superuser: read(1)?,
            bypass_rls: read(2)?,
            create_db: read(3)?,
            create_role: read(4)?,
            replication: read(5)?,
            rds_superuser: read(6)?,
            memberships: row
                .try_get::<_, Vec<String>>(7)
                .map_err(|error| super::describe_sqlstate(&error))?,
        })
    }

    #[must_use]
    pub fn elevated_attribute(&self) -> Option<&'static str> {
        if self.superuser {
            Some("SUPERUSER")
        } else if self.rds_superuser {
            Some("rds_superuser membership")
        } else if self.bypass_rls {
            Some("BYPASSRLS")
        } else {
            None
        }
    }

    pub fn enforce(&self, strict: bool) -> Result<()> {
        match (strict, self.elevated_attribute()) {
            (true, Some(attribute)) => Err(Error::RoleRefused {
                role: self.name.clone(),
                attribute: attribute.to_owned(),
            }),
            _ => Ok(()),
        }
    }

    #[must_use]
    pub fn warning(&self) -> Option<String> {
        self.elevated_attribute().map(|attribute| {
            format!(
                "the role `{}` has {attribute}; read-only mode rests on the parser and the transaction guard, not on the role",
                self.name
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(superuser: bool, bypass_rls: bool, rds: bool) -> RoleProfile {
        RoleProfile {
            name: "app".to_owned(),
            superuser,
            bypass_rls,
            create_db: false,
            create_role: false,
            replication: false,
            rds_superuser: rds,
            memberships: Vec::new(),
        }
    }

    #[test]
    fn strict_mode_refuses_every_elevated_attribute_and_names_it() {
        let error = role(true, false, false).enforce(true).unwrap_err();
        assert!(error.to_string().contains("SUPERUSER"));
        let error = role(false, true, false).enforce(true).unwrap_err();
        assert!(error.to_string().contains("BYPASSRLS"));
        let error = role(false, false, true).enforce(true).unwrap_err();
        assert!(error.to_string().contains("rds_superuser"));
        role(false, false, false).enforce(true).unwrap();
    }

    #[test]
    fn the_local_default_only_warns() {
        let elevated = role(true, false, false);
        elevated.enforce(false).unwrap();
        assert!(elevated.warning().unwrap().contains("parser"));
        assert!(role(false, false, false).warning().is_none());
    }
}
