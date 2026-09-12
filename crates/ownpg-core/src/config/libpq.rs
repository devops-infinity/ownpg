use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::environment::Environment;
use super::profile::{open_permissions, read_capped};
use super::{ChannelBinding, Secret, SslMode};
use crate::error::{Error, Result};

pub const LOCALHOST: &str = "localhost";
pub const DEFAULT_USER: &str = "postgres";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibpqLayer {
    pub host: Option<String>,
    pub hostaddr: Option<String>,
    pub port: Option<u16>,
    pub dbname: Option<String>,
    pub user: Option<String>,
    pub password: Option<Secret>,
    pub passfile: Option<PathBuf>,
    pub service: Option<String>,
    pub sslmode: Option<SslMode>,
    pub sslrootcert: Option<PathBuf>,
    pub sslcert: Option<PathBuf>,
    pub sslkey: Option<PathBuf>,
    pub connect_timeout: Option<Duration>,
    pub channel_binding: Option<ChannelBinding>,
    pub application_name: Option<String>,
    pub options: Option<String>,
    pub sslnegotiation: Option<String>,
}

impl LibpqLayer {
    #[must_use]
    pub fn from_environment(env: &Environment) -> Self {
        let mut pairs = BTreeMap::new();
        for (variable, key) in [
            ("PGHOST", "host"),
            ("PGHOSTADDR", "hostaddr"),
            ("PGPORT", "port"),
            ("PGDATABASE", "dbname"),
            ("PGUSER", "user"),
            ("PGPASSWORD", "password"),
            ("PGPASSFILE", "passfile"),
            ("PGSERVICE", "service"),
            ("PGSSLMODE", "sslmode"),
            ("PGSSLROOTCERT", "sslrootcert"),
            ("PGSSLCERT", "sslcert"),
            ("PGSSLKEY", "sslkey"),
            ("PGCONNECT_TIMEOUT", "connect_timeout"),
            ("PGCHANNELBINDING", "channel_binding"),
            ("PGAPPNAME", "application_name"),
            ("PGOPTIONS", "options"),
            ("PGSSLNEGOTIATION", "sslnegotiation"),
        ] {
            if let Some(value) = env.var(variable) {
                pairs.insert(key.to_owned(), value.to_owned());
            }
        }
        Self::from_pairs(&pairs).unwrap_or_default()
    }

    pub fn from_pairs(pairs: &BTreeMap<String, String>) -> Result<Self> {
        let mut layer = Self::default();
        for (key, value) in pairs {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match key.as_str() {
                "host" => layer.host = Some(single_host(value)?),
                "hostaddr" => layer.hostaddr = Some(single_host(value)?),
                "port" => layer.port = Some(parse_port(value)?),
                "dbname" => layer.dbname = Some(value.to_owned()),
                "user" => layer.user = Some(value.to_owned()),
                "password" => layer.password = Some(Secret::new(value.to_owned())),
                "passfile" => layer.passfile = Some(PathBuf::from(value)),
                "service" => layer.service = Some(value.to_owned()),
                "sslmode" => {
                    layer.sslmode = Some(SslMode::parse(value).ok_or_else(|| {
                        Error::ConfigInvalid {
                            setting: "sslmode".to_owned(),
                            value: value.to_owned(),
                            detail: "expected disable, allow, prefer, require, verify-ca, or verify-full".to_owned(),
                        }
                    })?);
                }
                "sslrootcert" => layer.sslrootcert = Some(PathBuf::from(value)),
                "sslcert" => layer.sslcert = Some(PathBuf::from(value)),
                "sslkey" => layer.sslkey = Some(PathBuf::from(value)),
                "connect_timeout" => {
                    let seconds: u64 = value.parse().map_err(|_| Error::ConfigInvalid {
                        setting: "connect_timeout".to_owned(),
                        value: value.to_owned(),
                        detail: "expected a whole number of seconds".to_owned(),
                    })?;
                    layer.connect_timeout = Some(Duration::from_secs(seconds));
                }
                "channel_binding" => {
                    layer.channel_binding = Some(ChannelBinding::parse(value).ok_or_else(
                        || Error::ConfigInvalid {
                            setting: "channel_binding".to_owned(),
                            value: value.to_owned(),
                            detail: "expected disable, prefer, or require".to_owned(),
                        },
                    )?);
                }
                "application_name" => layer.application_name = Some(value.to_owned()),
                "options" => layer.options = Some(value.to_owned()),
                "sslnegotiation" => layer.sslnegotiation = Some(value.to_owned()),
                _ => {}
            }
        }
        Ok(layer)
    }

    #[must_use]
    pub fn over(self, lower: Self) -> Self {
        Self {
            host: self.host.or(lower.host),
            hostaddr: self.hostaddr.or(lower.hostaddr),
            port: self.port.or(lower.port),
            dbname: self.dbname.or(lower.dbname),
            user: self.user.or(lower.user),
            password: self.password.or(lower.password),
            passfile: self.passfile.or(lower.passfile),
            service: self.service.or(lower.service),
            sslmode: self.sslmode.or(lower.sslmode),
            sslrootcert: self.sslrootcert.or(lower.sslrootcert),
            sslcert: self.sslcert.or(lower.sslcert),
            sslkey: self.sslkey.or(lower.sslkey),
            connect_timeout: self.connect_timeout.or(lower.connect_timeout),
            channel_binding: self.channel_binding.or(lower.channel_binding),
            application_name: self.application_name.or(lower.application_name),
            options: self.options.or(lower.options),
            sslnegotiation: self.sslnegotiation.or(lower.sslnegotiation),
        }
    }
}

fn single_host(value: &str) -> Result<String> {
    if value.contains(',') {
        return Err(Error::ConfigInvalid {
            setting: "host".to_owned(),
            value: value.to_owned(),
            detail: "one host per profile; a comma-separated list is not supported".to_owned(),
        });
    }
    Ok(value.to_owned())
}

fn parse_port(value: &str) -> Result<u16> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| Error::ConfigInvalid {
            setting: "port".to_owned(),
            value: value.to_owned(),
            detail: "expected a number from 1 to 65535".to_owned(),
        })
}

pub fn parse_dsn(text: &str) -> Result<LibpqLayer> {
    let text = text.trim();
    if text.starts_with("postgresql://") || text.starts_with("postgres://") {
        parse_uri(text)
    } else {
        parse_key_values(text)
    }
}

fn parse_uri(text: &str) -> Result<LibpqLayer> {
    let invalid = |detail: &str| Error::DsnInvalid {
        detail: detail.to_owned(),
    };
    let rest = text
        .strip_prefix("postgresql://")
        .or_else(|| text.strip_prefix("postgres://"))
        .ok_or_else(|| invalid("expected a postgresql:// or postgres:// prefix"))?;
    let (before_query, query) = match rest.split_once('?') {
        Some((head, query)) => (head, Some(query)),
        None => (rest, None),
    };
    let (authority, path) = match before_query.split_once('/') {
        Some((authority, path)) => (authority, Some(path)),
        None => (before_query, None),
    };
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((userinfo, hostport)) => (Some(userinfo), hostport),
        None => (None, authority),
    };
    let mut pairs: BTreeMap<String, String> = BTreeMap::new();
    if let Some(userinfo) = userinfo {
        let (user, password) = match userinfo.split_once(':') {
            Some((user, password)) => (user, Some(password)),
            None => (userinfo, None),
        };
        if !user.is_empty() {
            pairs.insert("user".to_owned(), percent_decode(user)?);
        }
        if let Some(password) = password {
            pairs.insert("password".to_owned(), percent_decode(password)?);
        }
    }
    if !hostport.is_empty() {
        let (host, port) = if let Some(closing) = hostport.strip_prefix('[') {
            let (host, tail) = closing
                .split_once(']')
                .ok_or_else(|| invalid("an IPv6 host needs a closing bracket"))?;
            (host.to_owned(), tail.strip_prefix(':'))
        } else {
            match hostport.rsplit_once(':') {
                Some((host, port)) => (host.to_owned(), Some(port)),
                None => (hostport.to_owned(), None),
            }
        };
        if !host.is_empty() {
            pairs.insert("host".to_owned(), percent_decode(&host)?);
        }
        if let Some(port) = port.filter(|port| !port.is_empty()) {
            pairs.insert("port".to_owned(), port.to_owned());
        }
    }
    if let Some(path) = path.filter(|path| !path.is_empty()) {
        pairs.insert("dbname".to_owned(), percent_decode(path)?);
    }
    if let Some(query) = query {
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| invalid("a query parameter needs a key and a value"))?;
            pairs.insert(percent_decode(key)?, percent_decode(value)?);
        }
    }
    LibpqLayer::from_pairs(&pairs)
}

fn parse_key_values(text: &str) -> Result<LibpqLayer> {
    let invalid = |detail: &str| Error::DsnInvalid {
        detail: detail.to_owned(),
    };
    let mut pairs: BTreeMap<String, String> = BTreeMap::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c.is_whitespace() {
                break;
            }
            key.push(c);
            chars.next();
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.next() != Some('=') {
            return Err(invalid("expected `key=value` pairs separated by spaces"));
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut value = String::new();
        if chars.peek() == Some(&'\'') {
            chars.next();
            loop {
                match chars.next() {
                    Some('\\') => match chars.next() {
                        Some(escaped) => value.push(escaped),
                        None => return Err(invalid("a quoted value ended inside an escape")),
                    },
                    Some('\'') => break,
                    Some(c) => value.push(c),
                    None => return Err(invalid("a quoted value has no closing quote")),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                if c == '\\' {
                    chars.next();
                    match chars.next() {
                        Some(escaped) => value.push(escaped),
                        None => return Err(invalid("a value ended inside an escape")),
                    }
                    continue;
                }
                value.push(c);
                chars.next();
            }
        }
        if key.is_empty() {
            return Err(invalid("a pair has an empty key"));
        }
        pairs.insert(key, value);
    }
    LibpqLayer::from_pairs(&pairs)
}

fn percent_decode(text: &str) -> Result<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes.get(index).copied().unwrap_or(0);
        if byte == b'%' {
            let pair = bytes
                .get(index + 1..index + 3)
                .and_then(|pair| std::str::from_utf8(pair).ok())
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| Error::DsnInvalid {
                    detail: "a percent escape needs two hex digits".to_owned(),
                })?;
            out.push(pair);
            index += 3;
        } else {
            out.push(byte);
            index += 1;
        }
    }
    String::from_utf8(out).map_err(|_| Error::DsnInvalid {
        detail: "a percent escape produced text that is not UTF-8".to_owned(),
    })
}

pub fn service_file_paths(env: &Environment) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    match env.var("PGSERVICEFILE") {
        Some(path) => paths.push(PathBuf::from(path)),
        None => {
            if let Some(path) = env.in_home(".pg_service.conf") {
                paths.push(path);
            }
        }
    }
    if let Some(dir) = env.var("PGSYSCONFDIR") {
        paths.push(Path::new(dir).join("pg_service.conf"));
    }
    paths
}

pub fn service_section(env: &Environment, name: &str) -> Result<Option<LibpqLayer>> {
    for path in service_file_paths(env) {
        if !path.exists() {
            continue;
        }
        let text = read_capped(&path)?;
        if let Some(pairs) = section_of(&text, name) {
            return LibpqLayer::from_pairs(&pairs).map(Some);
        }
    }
    Ok(None)
}

fn section_of(text: &str, wanted: &str) -> Option<BTreeMap<String, String>> {
    let mut inside = false;
    let mut pairs = BTreeMap::new();
    let mut found = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            if inside {
                break;
            }
            inside = name.trim() == wanted;
            found |= inside;
            continue;
        }
        if inside && let Some((key, value)) = line.split_once('=') {
            pairs.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    found.then_some(pairs)
}

pub fn password_file_path(env: &Environment, layer_passfile: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = layer_passfile {
        return Some(path.to_path_buf());
    }
    if cfg!(windows) {
        env.var("APPDATA")
            .map(|appdata| Path::new(appdata).join("postgresql").join("pgpass.conf"))
    } else {
        env.in_home(".pgpass")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordFileOutcome {
    Found(Secret),
    NoMatch,
    Missing,
    IgnoredPermissions { path: PathBuf, mode: u32 },
}

pub fn password_from_file(
    path: &Path,
    host: Option<&str>,
    port: u16,
    database: &str,
    user: &str,
) -> Result<PasswordFileOutcome> {
    if !path.exists() {
        return Ok(PasswordFileOutcome::Missing);
    }
    if let Some(mode) = open_permissions(path)? {
        return Ok(PasswordFileOutcome::IgnoredPermissions {
            path: path.to_path_buf(),
            mode,
        });
    }
    let text = read_capped(path)?;
    let host_key = match host {
        Some(host) if !super::presets::is_socket_directory(host) => host,
        _ => LOCALHOST,
    };
    let port_text = port.to_string();
    for raw in text.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let fields = split_password_line(line);
        if fields.len() < 5 {
            continue;
        }
        let matches = |field: &str, want: &str| field == "*" || field == want;
        let (Some(h), Some(p), Some(d), Some(u)) =
            (fields.first(), fields.get(1), fields.get(2), fields.get(3))
        else {
            continue;
        };
        if matches(h, host_key)
            && matches(p, &port_text)
            && matches(d, database)
            && matches(u, user)
        {
            let password = fields
                .get(4..)
                .map(|tail| tail.join(":"))
                .unwrap_or_default();
            return Ok(PasswordFileOutcome::Found(Secret::new(password)));
        }
    }
    Ok(PasswordFileOutcome::NoMatch)
}

fn split_password_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ':' => {
                fields.push(std::mem::take(&mut current));
            }
            other => current.push(other),
        }
    }
    fields.push(current);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorId;
    use std::fs;

    #[test]
    fn the_environment_layer_reads_every_documented_variable() {
        let env = Environment::default()
            .with_var("PGHOST", "db.internal")
            .with_var("PGPORT", "5433")
            .with_var("PGDATABASE", "app")
            .with_var("PGUSER", "reader")
            .with_var("PGSSLMODE", "verify-full")
            .with_var("PGCONNECT_TIMEOUT", "7")
            .with_var("PGCHANNELBINDING", "require")
            .with_var("PGAPPNAME", "custom");
        let layer = LibpqLayer::from_environment(&env);
        assert_eq!(layer.host.as_deref(), Some("db.internal"));
        assert_eq!(layer.port, Some(5433));
        assert_eq!(layer.dbname.as_deref(), Some("app"));
        assert_eq!(layer.user.as_deref(), Some("reader"));
        assert_eq!(layer.sslmode, Some(SslMode::VerifyFull));
        assert_eq!(layer.connect_timeout, Some(Duration::from_secs(7)));
        assert_eq!(layer.channel_binding, Some(ChannelBinding::Require));
        assert_eq!(layer.application_name.as_deref(), Some("custom"));
    }

    #[test]
    fn a_uri_dsn_yields_every_component() {
        let layer = parse_dsn(
            "postgresql://rea%20der:p%40ss@db.internal:5433/app?sslmode=require&connect_timeout=3",
        )
        .unwrap();
        assert_eq!(layer.user.as_deref(), Some("rea der"));
        assert_eq!(layer.password.as_ref().map(Secret::expose), Some("p@ss"));
        assert_eq!(layer.host.as_deref(), Some("db.internal"));
        assert_eq!(layer.port, Some(5433));
        assert_eq!(layer.dbname.as_deref(), Some("app"));
        assert_eq!(layer.sslmode, Some(SslMode::Require));
        assert_eq!(layer.connect_timeout, Some(Duration::from_secs(3)));
    }

    #[test]
    fn an_ipv6_uri_host_keeps_its_brackets_out_of_the_host() {
        let layer = parse_dsn("postgres://[::1]:5432/app").unwrap();
        assert_eq!(layer.host.as_deref(), Some("::1"));
        assert_eq!(layer.port, Some(5432));
    }

    #[test]
    fn a_socket_uri_with_a_percent_encoded_directory_works() {
        let layer = parse_dsn("postgresql://%2Ftmp/app").unwrap();
        assert_eq!(layer.host.as_deref(), Some("/tmp"));
        assert_eq!(layer.dbname.as_deref(), Some("app"));
    }

    #[test]
    fn a_key_value_dsn_handles_quotes_and_escapes() {
        let layer =
            parse_dsn("host=/tmp dbname=app user=u password='it\\'s' options='-c search_path=x'")
                .unwrap();
        assert_eq!(layer.host.as_deref(), Some("/tmp"));
        assert_eq!(layer.dbname.as_deref(), Some("app"));
        assert_eq!(layer.password.as_ref().map(Secret::expose), Some("it's"));
        assert_eq!(layer.options.as_deref(), Some("-c search_path=x"));
    }

    #[test]
    fn a_broken_dsn_is_refused_with_the_dsn_id() {
        assert_eq!(
            parse_dsn("postgresql://[::1/app").unwrap_err().id(),
            ErrorId::DsnInvalid
        );
        assert_eq!(parse_dsn("host").unwrap_err().id(), ErrorId::DsnInvalid);
        assert_eq!(
            parse_dsn("postgresql://a%zz@h/d").unwrap_err().id(),
            ErrorId::DsnInvalid
        );
    }

    #[test]
    fn a_host_list_is_refused() {
        let error = parse_dsn("host=a,b dbname=x").unwrap_err();
        assert_eq!(error.id(), ErrorId::ConfigInvalid);
    }

    #[test]
    fn the_upper_layer_wins_field_by_field() {
        let lower = LibpqLayer {
            host: Some("lower".to_owned()),
            port: Some(1),
            ..LibpqLayer::default()
        };
        let upper = LibpqLayer {
            host: Some("upper".to_owned()),
            ..LibpqLayer::default()
        };
        let merged = upper.over(lower);
        assert_eq!(merged.host.as_deref(), Some("upper"));
        assert_eq!(merged.port, Some(1));
    }

    #[test]
    fn a_service_section_is_read_from_the_user_file_before_the_system_file() {
        let dir = tempfile::tempdir().unwrap();
        let user_file = dir.path().join("user.conf");
        fs::write(&user_file, "# user\n[staging]\nhost=user-host\nport=5433\n").unwrap();
        let system_dir = dir.path().join("sys");
        fs::create_dir_all(&system_dir).unwrap();
        fs::write(
            system_dir.join("pg_service.conf"),
            "[staging]\nhost=system-host\n[other]\nhost=other-host\n",
        )
        .unwrap();
        let env = Environment::default()
            .with_var("PGSERVICEFILE", user_file.to_str().unwrap())
            .with_var("PGSYSCONFDIR", system_dir.to_str().unwrap());
        let staging = service_section(&env, "staging").unwrap().unwrap();
        assert_eq!(staging.host.as_deref(), Some("user-host"));
        assert_eq!(staging.port, Some(5433));
        let other = service_section(&env, "other").unwrap().unwrap();
        assert_eq!(other.host.as_deref(), Some("other-host"));
        assert!(service_section(&env, "missing").unwrap().is_none());
    }

    #[cfg(unix)]
    fn private_file(dir: &Path, name: &str, text: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn the_password_file_matches_by_field_with_wildcards_and_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let path = private_file(
            dir.path(),
            "pgpass",
            "# comment\ndb.internal:5432:app:reader:first\n*:*:app:writer:se\\:cret\nlocalhost:*:*:*:local\n",
        );
        let found = password_from_file(&path, Some("db.internal"), 5432, "app", "reader").unwrap();
        assert_eq!(
            found,
            PasswordFileOutcome::Found(Secret::new("first".to_owned()))
        );
        let escaped = password_from_file(&path, Some("other"), 5433, "app", "writer").unwrap();
        assert_eq!(
            escaped,
            PasswordFileOutcome::Found(Secret::new("se:cret".to_owned()))
        );
        let socket = password_from_file(&path, None, 5432, "any", "anyone").unwrap();
        assert_eq!(
            socket,
            PasswordFileOutcome::Found(Secret::new("local".to_owned()))
        );
        let socket_dir = password_from_file(&path, Some("/tmp"), 5432, "any", "anyone").unwrap();
        assert_eq!(
            socket_dir,
            PasswordFileOutcome::Found(Secret::new("local".to_owned()))
        );
        let none = password_from_file(&path, Some("nope"), 1, "x", "y").unwrap();
        assert_eq!(none, PasswordFileOutcome::NoMatch);
    }

    #[cfg(unix)]
    #[test]
    fn a_password_file_readable_by_others_is_ignored_not_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = private_file(dir.path(), "pgpass", "*:*:*:*:pw\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let outcome = password_from_file(&path, None, 5432, "a", "b").unwrap();
        assert!(matches!(
            outcome,
            PasswordFileOutcome::IgnoredPermissions { mode: 0o644, .. }
        ));
    }

    #[test]
    fn a_missing_password_file_is_reported_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = password_from_file(&dir.path().join("none"), None, 5432, "a", "b").unwrap();
        assert_eq!(outcome, PasswordFileOutcome::Missing);
    }

    #[test]
    fn the_password_file_path_honors_the_layer_then_the_home() {
        let env = Environment::default().with_home("/home/u".into());
        assert_eq!(
            password_file_path(&env, Some(Path::new("/x/pgpass"))),
            Some(PathBuf::from("/x/pgpass"))
        );
        if cfg!(unix) {
            assert_eq!(
                password_file_path(&env, None),
                Some(PathBuf::from("/home/u/.pgpass"))
            );
        }
    }
}
