use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::read::facts_for;
use super::{Call, Outcome, Route, ToolFailure, ToolOutput, route};
use crate::error::{Error, Result};
use crate::render::{quote_literal, verify};
use crate::shape::ResultSet;
use crate::tool_specs;

const ACTIVITY_DESCRIPTION: &str = "List client backends from pg_stat_activity: pid, role, application, client address, state, wait event, how long the transaction and the current statement have run, and the statement text. Idle sessions are left out unless include_idle is true; min_duration_seconds keeps only statements running at least that long. Sorted by transaction start, oldest first, then pid.";

const LOCKS_DESCRIPTION: &str = "List lock waits and the blocking chain behind them: for every backend waiting on a lock, the pids blocking it (pg_blocking_pids), the lock type, the relation, the mode, how long it has waited, and the statements on both sides. Sorted by wait duration, longest first, then pid.";

const REPLICATION_DESCRIPTION: &str = "Report replication: whether this server is in recovery, the connected standbys from pg_stat_replication with their state, sync state, and lag in bytes, and the replication slots with their type, activity, and retained WAL. Sorted by client name, then slot name.";

const WAL_DESCRIPTION: &str = "Report WAL and checkpoint activity: the current WAL position, wal records, full page images, and bytes from pg_stat_wal, checkpoint counts and timing from pg_stat_checkpointer (PostgreSQL 17 and later) or pg_stat_bgwriter, and I/O totals from pg_stat_io (PostgreSQL 16 and later). One row per source, sorted by source.";

const INDEXES_HEALTH_DESCRIPTION: &str = "Find indexes in the scoped schema that need attention: invalid ones left by a failed concurrent build, duplicates that cover the same columns as another index on the same table, and unused ones with zero scans since the statistics reset that are neither unique nor a primary key. Each row names the problem, the table, the index, and its size. Sorted by problem, then size largest first, then index name.";

const BLOAT_DESCRIPTION: &str = "Estimate table and B-tree index bloat in the scoped schema from pg_class and pg_stats without scanning the data. is_na marks rows the estimator cannot judge (no statistics or unsupported types). Name exact_table to scan one table and its B-tree indexes with pgstattuple for exact numbers; that needs the extension installed and fails with extension.missing otherwise. Sorted by wasted bytes, largest first, then name.";

const SETTINGS_DESCRIPTION: &str = "List server settings from pg_settings with value, unit, source, and whether a restart is pending. pattern filters names with LIKE; changed_only keeps settings that differ from their default. Sorted by name.";

const TOP_QUERIES_DESCRIPTION: &str = "List the statements with the highest cost from pg_stat_statements: calls, total and mean execution time, rows, shared block hits and reads, and the normalized statement. order_by picks the ranking column. The extension must be installed in this database; the tool says so when it is absent. Sorted by the chosen column, largest first, then queryid.";

async fn run_catalog(call: &Call, operation: &str, sql: &str, row_cap: u32) -> Outcome {
    let classification = verify(sql, &["SelectStmt"])?;
    let mut facts = facts_for(&classification);
    facts.operation = Some(operation.to_owned());
    let result = call
        .engine()
        .run_read(sql, call.caps(row_cap))
        .await
        .map_err(|error| ToolFailure::from(error).with_facts(facts.clone()))?;
    let facts = facts.with_result(&result);
    let text = result.render_text();
    Ok(ToolOutput::structured(&result, text)?
        .with_facts(facts)
        .into())
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityArgs {
    #[serde(default)]
    pub include_idle: bool,
    #[serde(default)]
    #[schemars(
        description = "Keep statements running at least this many seconds; 0 keeps every one."
    )]
    pub min_duration_seconds: u64,
    #[serde(default)]
    #[schemars(
        description = "Rows per page. 0 uses the configured default (100). The maximum is 1000."
    )]
    pub row_cap: u32,
}

fn activity_sql(args: &ActivityArgs) -> String {
    let idle = if args.include_idle {
        ""
    } else {
        " AND state IS DISTINCT FROM 'idle'"
    };
    format!(
        "SELECT pid, usename::text AS role, application_name, client_addr::text AS client, backend_type, state, \
             wait_event_type, wait_event, \
             EXTRACT(EPOCH FROM (now() - xact_start))::int8 AS transaction_seconds, \
             EXTRACT(EPOCH FROM (now() - query_start))::int8 AS statement_seconds, \
             left(query, 500) AS query \
             FROM pg_catalog.pg_stat_activity \
             WHERE backend_type = 'client backend' AND pid <> pg_catalog.pg_backend_pid(){idle} \
             AND COALESCE(EXTRACT(EPOCH FROM (now() - query_start)), 0) >= {} \
             ORDER BY xact_start NULLS LAST, pid",
        args.min_duration_seconds
    )
}

pub fn activity(call: Call, args: ActivityArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let sql = activity_sql(&args);
        run_catalog(&call, "activity", &sql, args.row_cap).await
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocksArgs {
    #[serde(default)]
    #[schemars(
        description = "Rows per page. 0 uses the configured default (100). The maximum is 1000."
    )]
    pub row_cap: u32,
}

const LOCKS_SQL: &str = "SELECT a.pid, pg_catalog.pg_blocking_pids(a.pid)::text AS blocked_by, l.locktype, \
                   l.relation::regclass::text AS relation, l.mode, \
                   EXTRACT(EPOCH FROM (now() - a.query_start))::int8 AS waiting_seconds, \
                   left(a.query, 300) AS query, \
                   (SELECT string_agg(left(b.query, 200), ' | ') FROM pg_catalog.pg_stat_activity b \
                    WHERE b.pid = ANY(pg_catalog.pg_blocking_pids(a.pid))) AS blocking_queries \
                   FROM pg_catalog.pg_stat_activity a \
                   JOIN pg_catalog.pg_locks l ON l.pid = a.pid AND NOT l.granted \
                   WHERE cardinality(pg_catalog.pg_blocking_pids(a.pid)) > 0 \
                   ORDER BY waiting_seconds DESC NULLS LAST, a.pid";

pub fn locks(call: Call, args: LocksArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move { run_catalog(&call, "locks", LOCKS_SQL, args.row_cap).await })
}

const REPLICATION_SQL: &str = "SELECT 'server' AS kind, 'in_recovery' AS name, pg_catalog.pg_is_in_recovery()::text AS state, NULL::text AS sync_state, \
                   CASE WHEN pg_catalog.pg_is_in_recovery() THEN pg_catalog.pg_wal_lsn_diff(pg_catalog.pg_last_wal_receive_lsn(), pg_catalog.pg_last_wal_replay_lsn())::int8 ELSE 0 END AS lag_bytes, \
                   NULL::text AS detail \
                   UNION ALL \
                   SELECT 'standby', COALESCE(client_addr::text, application_name), state, sync_state, \
                   pg_catalog.pg_wal_lsn_diff(pg_catalog.pg_current_wal_lsn(), replay_lsn)::int8, \
                   'write lag ' || COALESCE(write_lag::text, '-') || ', flush lag ' || COALESCE(flush_lag::text, '-') || ', replay lag ' || COALESCE(replay_lag::text, '-') \
                   FROM pg_catalog.pg_stat_replication \
                   UNION ALL \
                   SELECT 'slot', slot_name::text, CASE WHEN active THEN 'active' ELSE 'inactive' END, slot_type::text, \
                   CASE WHEN pg_catalog.pg_is_in_recovery() THEN NULL ELSE pg_catalog.pg_wal_lsn_diff(pg_catalog.pg_current_wal_lsn(), restart_lsn)::int8 END, \
                   COALESCE(plugin::text, '') || CASE WHEN database IS NULL THEN '' ELSE ' database ' || database::text END \
                   FROM pg_catalog.pg_replication_slots \
                   ORDER BY 1, 2";

pub fn replication(call: Call, _args: super::health::NoArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move { run_catalog(&call, "replication", REPLICATION_SQL, 1_000).await })
}

fn wal_sql(features: crate::engine::Features) -> String {
    let checkpointer = if features.supports_stat_checkpointer() {
        "SELECT 'checkpointer' AS source, 'timed checkpoints' AS metric, num_timed::text AS value FROM pg_catalog.pg_stat_checkpointer \
             UNION ALL SELECT 'checkpointer', 'requested checkpoints', num_requested::text FROM pg_catalog.pg_stat_checkpointer \
             UNION ALL SELECT 'checkpointer', 'write time ms', write_time::text FROM pg_catalog.pg_stat_checkpointer \
             UNION ALL SELECT 'checkpointer', 'sync time ms', sync_time::text FROM pg_catalog.pg_stat_checkpointer \
             UNION ALL SELECT 'checkpointer', 'buffers written', buffers_written::text FROM pg_catalog.pg_stat_checkpointer"
    } else {
        "SELECT 'bgwriter' AS source, 'timed checkpoints' AS metric, checkpoints_timed::text AS value FROM pg_catalog.pg_stat_bgwriter \
             UNION ALL SELECT 'bgwriter', 'requested checkpoints', checkpoints_req::text FROM pg_catalog.pg_stat_bgwriter \
             UNION ALL SELECT 'bgwriter', 'write time ms', checkpoint_write_time::text FROM pg_catalog.pg_stat_bgwriter \
             UNION ALL SELECT 'bgwriter', 'sync time ms', checkpoint_sync_time::text FROM pg_catalog.pg_stat_bgwriter \
             UNION ALL SELECT 'bgwriter', 'buffers written by checkpoints', buffers_checkpoint::text FROM pg_catalog.pg_stat_bgwriter"
    };
    let io = if features.supports_stat_io() {
        " UNION ALL SELECT 'io', backend_type || ' ' || object || ' ' || context || ' reads', sum(reads)::text FROM pg_catalog.pg_stat_io WHERE reads > 0 GROUP BY backend_type, object, context \
              UNION ALL SELECT 'io', backend_type || ' ' || object || ' ' || context || ' writes', sum(writes)::text FROM pg_catalog.pg_stat_io WHERE writes > 0 GROUP BY backend_type, object, context"
    } else {
        ""
    };
    format!(
        "SELECT * FROM ( \
             SELECT 'wal' AS source, 'current lsn' AS metric, pg_catalog.pg_current_wal_lsn()::text AS value \
             UNION ALL SELECT 'wal', 'records', wal_records::text FROM pg_catalog.pg_stat_wal \
             UNION ALL SELECT 'wal', 'full page images', wal_fpi::text FROM pg_catalog.pg_stat_wal \
             UNION ALL SELECT 'wal', 'bytes', wal_bytes::text FROM pg_catalog.pg_stat_wal \
             UNION ALL SELECT 'wal', 'buffers full', wal_buffers_full::text FROM pg_catalog.pg_stat_wal \
             UNION ALL {checkpointer}{io} \
             ) AS metrics ORDER BY source, metric"
    )
}

pub fn wal(call: Call, _args: super::health::NoArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let sql = wal_sql(call.engine().features());
        run_catalog(&call, "wal", &sql, 1_000).await
    })
}

pub(crate) fn indexes_health_sql(schema: &str) -> String {
    let scoped = quote_literal(schema);
    format!(
        "SELECT * FROM ( \
             SELECT 'invalid' AS problem, c.relname::text AS table_name, i.relname::text AS index_name, \
             pg_catalog.pg_relation_size(i.oid)::int8 AS size_bytes, 'rebuild it with pg_index reindex, or drop it and create it again' AS advice \
             FROM pg_catalog.pg_index x JOIN pg_catalog.pg_class i ON i.oid = x.indexrelid JOIN pg_catalog.pg_class c ON c.oid = x.indrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = {scoped} AND NOT x.indisvalid \
             UNION ALL \
             SELECT 'duplicate', c.relname::text, i.relname::text, pg_catalog.pg_relation_size(i.oid)::int8, \
             'same columns as ' || string_agg(o.relname::text, ', ') \
             FROM pg_catalog.pg_index x JOIN pg_catalog.pg_class i ON i.oid = x.indexrelid JOIN pg_catalog.pg_class c ON c.oid = x.indrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_catalog.pg_index y ON y.indrelid = x.indrelid AND y.indexrelid <> x.indexrelid \
               AND y.indkey::text = x.indkey::text AND y.indclass::text = x.indclass::text \
               AND COALESCE(pg_catalog.pg_get_expr(y.indpred, y.indrelid), '') = COALESCE(pg_catalog.pg_get_expr(x.indpred, x.indrelid), '') \
               AND COALESCE(pg_catalog.pg_get_expr(y.indexprs, y.indrelid), '') = COALESCE(pg_catalog.pg_get_expr(x.indexprs, x.indrelid), '') \
             JOIN pg_catalog.pg_class o ON o.oid = y.indexrelid \
             WHERE n.nspname = {scoped} AND NOT x.indisprimary AND i.relname > o.relname \
             GROUP BY c.relname, i.relname, i.oid \
             UNION ALL \
             SELECT 'unused', c.relname::text, i.relname::text, pg_catalog.pg_relation_size(i.oid)::int8, \
             'zero scans since the statistics reset; check the workload before dropping it' \
             FROM pg_catalog.pg_stat_user_indexes s JOIN pg_catalog.pg_index x ON x.indexrelid = s.indexrelid \
             JOIN pg_catalog.pg_class i ON i.oid = s.indexrelid JOIN pg_catalog.pg_class c ON c.oid = s.relid \
             WHERE s.schemaname = {scoped} AND s.idx_scan = 0 AND NOT x.indisunique AND NOT x.indisprimary AND x.indisvalid \
             ) AS findings ORDER BY problem, size_bytes DESC, index_name"
    )
}

pub fn indexes_health(call: Call, _args: super::health::NoArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let sql = indexes_health_sql(&call.settings().schema.value);
        run_catalog(&call, "indexes_health", &sql, 1_000).await
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BloatArgs {
    #[serde(default)]
    #[schemars(
        description = "A table in the scoped schema to measure exactly with pgstattuple, which scans the table and its B-tree indexes; empty runs the estimator over the whole schema."
    )]
    pub exact_table: String,
    #[serde(default)]
    #[schemars(
        description = "Rows per page. 0 uses the configured default (100). The maximum is 1000."
    )]
    pub row_cap: u32,
}

async fn extension_schema(call: &Call, name: &str) -> Result<String> {
    Ok(installed_extension(call, name).await?.0)
}

async fn installed_extension(call: &Call, name: &str) -> Result<(String, String)> {
    let rows = call
        .engine()
        .catalog_rows(
            "SELECT n.nspname::text, e.extversion::text FROM pg_catalog.pg_extension e JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace WHERE e.extname = $1",
            &[&name],
        )
        .await?;
    let Some(row) = rows.first() else {
        return Err(Error::ExtensionMissing {
            name: name.to_owned(),
        });
    };
    Ok((
        super::catalog::read_column::<String>(row, 0)?,
        super::catalog::read_column::<String>(row, 1)?,
    ))
}

fn version_at_least(installed: &str, needed: (u32, u32)) -> bool {
    let mut parts = installed.split('.').map(|part| part.parse::<u32>().ok());
    let major = parts.next().flatten();
    let minor = parts.next().flatten().unwrap_or(0);
    major.is_some_and(|major| (major, minor) >= needed)
}

const STAT_STATEMENTS_NEEDED: &str = "1.8";

fn exact_bloat_sql(extension_schema: &str, table: &crate::render::QualifiedName) -> String {
    let extension = crate::render::quote_ident(extension_schema);
    let relation = quote_literal(&table.sql());
    let name = quote_literal(&table.name);
    format!(
        "SELECT * FROM ( \
             SELECT 'table' AS kind, {name}::text AS name, table_len::int8 AS real_bytes, (dead_tuple_len + free_space)::int8 AS wasted_bytes, \
             round((dead_tuple_percent + free_percent)::numeric, 1) AS wasted_percent, false AS is_na \
             FROM {extension}.pgstattuple({relation}::regclass) \
             UNION ALL \
             SELECT 'index', i.relname::text, s.index_size::int8, \
             (s.leaf_pages * pg_catalog.current_setting('block_size')::int8 * (100 - s.avg_leaf_density) / 100)::int8, \
             round((100 - s.avg_leaf_density)::numeric, 1), false \
             FROM pg_catalog.pg_index x JOIN pg_catalog.pg_class i ON i.oid = x.indexrelid \
             JOIN pg_catalog.pg_am am ON am.oid = i.relam \
             CROSS JOIN LATERAL {extension}.pgstatindex(i.oid::regclass) AS s \
             WHERE x.indrelid = {relation}::regclass AND am.amname = 'btree' AND x.indisvalid \
             ) AS bloat ORDER BY wasted_bytes DESC, name"
    )
}

pub(crate) fn bloat_sql(schema: &str) -> String {
    let scoped = quote_literal(schema);
    format!(
        "SELECT * FROM ( \
             SELECT 'table' AS kind, tblname AS name, bs * tblpages AS real_bytes, \
             GREATEST((tblpages - est_tblpages_ff) * bs, 0)::int8 AS wasted_bytes, \
             CASE WHEN tblpages > 0 AND tblpages - est_tblpages_ff > 0 THEN round((100.0 * (tblpages - est_tblpages_ff) / tblpages)::numeric, 1) ELSE 0 END AS wasted_percent, \
             is_na \
             FROM ( \
               SELECT ceil(reltuples / ((bs - page_hdr) * fillfactor / (tpl_size * 100))) + ceil(toasttuples / 4) AS est_tblpages_ff, \
               tblpages, fillfactor, bs, tblname, is_na \
               FROM ( \
                 SELECT (4 + tpl_hdr_size + tpl_data_size + (2 * ma) - CASE WHEN tpl_hdr_size % ma = 0 THEN ma ELSE tpl_hdr_size % ma END \
                        - CASE WHEN ceil(tpl_data_size)::int % ma = 0 THEN ma ELSE ceil(tpl_data_size)::int % ma END) AS tpl_size, \
                        bs - page_hdr AS size_per_block, (heappages + toastpages) AS tblpages, heappages, toastpages, reltuples, toasttuples, bs, page_hdr, tblname, fillfactor, is_na \
                 FROM ( \
                   SELECT tbl.relname::text AS tblname, tbl.reltuples, tbl.relpages AS heappages, COALESCE(toast.relpages, 0) AS toastpages, \
                          COALESCE(toast.reltuples, 0) AS toasttuples, \
                          COALESCE(substring(array_to_string(tbl.reloptions, ' ') FROM 'fillfactor=([0-9]+)')::smallint, 100) AS fillfactor, \
                          current_setting('block_size')::numeric AS bs, \
                          CASE WHEN version() ~ 'mingw32' OR version() ~ '64-bit|x86_64|ppc64|ia64|amd64|aarch64' THEN 8 ELSE 4 END AS ma, \
                          24 AS page_hdr, \
                          23 + CASE WHEN MAX(COALESCE(s.null_frac, 0)) > 0 THEN (7 + count(s.attname)) / 8 ELSE 0::int END \
                             + CASE WHEN bool_or(att.attname = 'oid' AND att.attnum < 0) THEN 4 ELSE 0 END AS tpl_hdr_size, \
                          sum((1 - COALESCE(s.null_frac, 0)) * COALESCE(s.avg_width, 0)) AS tpl_data_size, \
                          bool_or(att.atttypid = 'pg_catalog.name'::regtype) OR sum(CASE WHEN att.attnum > 0 THEN 1 ELSE 0 END) <> count(s.attname) AS is_na \
                   FROM pg_catalog.pg_attribute AS att \
                   JOIN pg_catalog.pg_class AS tbl ON att.attrelid = tbl.oid \
                   JOIN pg_catalog.pg_namespace AS ns ON ns.oid = tbl.relnamespace \
                   LEFT JOIN pg_catalog.pg_stats AS s ON s.schemaname = ns.nspname AND s.tablename = tbl.relname AND s.inherited = false AND s.attname = att.attname \
                   LEFT JOIN pg_catalog.pg_class AS toast ON tbl.reltoastrelid = toast.oid \
                   WHERE NOT att.attisdropped AND tbl.relkind IN ('r', 'm') AND ns.nspname = {scoped} \
                   GROUP BY 1, 2, 3, 4, 5, 6, 7, 8, 9 \
                 ) AS s \
               ) AS s2 \
             ) AS s3 \
             UNION ALL \
             SELECT 'index', idxname, bs * relpages, \
             GREATEST(bs * (relpages - est_pages_ff), 0)::int8, \
             CASE WHEN relpages > 0 AND relpages - est_pages_ff > 0 THEN round((100.0 * (relpages - est_pages_ff) / relpages)::numeric, 1) ELSE 0 END, \
             is_na \
             FROM ( \
               SELECT idxname, bs, relpages, is_na, \
               coalesce(1 + ceil(reltuples / floor((bs - pageopqdata - pagehdr) * fillfactor / (100 * (4 + nulldatahdrwidth)::float))), 0) AS est_pages_ff \
               FROM ( \
                 SELECT idxname, reltuples, relpages, fillfactor, bs, is_na, \
                        ( index_tuple_hdr_bm + maxalign - CASE WHEN index_tuple_hdr_bm % maxalign = 0 THEN maxalign ELSE index_tuple_hdr_bm % maxalign END \
                        + nulldatawidth + maxalign - CASE WHEN nulldatawidth = 0 THEN 0 WHEN nulldatawidth::integer % maxalign = 0 THEN maxalign ELSE nulldatawidth::integer % maxalign END )::numeric AS nulldatahdrwidth, \
                        pagehdr, pageopqdata \
                 FROM ( \
                   SELECT i.idxname, i.reltuples, i.relpages, i.fillfactor, current_setting('block_size')::numeric AS bs, \
                          CASE WHEN version() ~ 'mingw32' OR version() ~ '64-bit|x86_64|ppc64|ia64|amd64|aarch64' THEN 8 ELSE 4 END AS maxalign, \
                          24 AS pagehdr, 16 AS pageopqdata, \
                          CASE WHEN max(coalesce(s.null_frac, 0)) = 0 THEN 8 ELSE 8 + ((32 + 8 - 1) / 8) END AS index_tuple_hdr_bm, \
                          sum((1.0 - coalesce(s.null_frac, 0.0)) * coalesce(s.avg_width, 1024)) AS nulldatawidth, \
                          max(CASE WHEN i.atttypid = 'pg_catalog.name'::regtype THEN 1 ELSE 0 END) > 0 AS is_na \
                   FROM ( \
                     SELECT ct.relname AS tblname, ct.relnamespace, ic.idxname, ic.attpos, ic.indkey, ic.indkey[ic.attpos] AS keyno, ic.reltuples, ic.relpages, ic.tbloid, ic.idxoid, ic.fillfactor, \
                            coalesce(a1.attnum, a2.attnum) AS attnum, coalesce(a1.attname, a2.attname) AS attname, coalesce(a1.atttypid, a2.atttypid) AS atttypid, \
                            CASE WHEN a1.attnum IS NULL THEN ic.idxname ELSE ct.relname END AS attrelname \
                     FROM ( \
                       SELECT idxname, reltuples, relpages, tbloid, idxoid, fillfactor, indkey, pg_catalog.generate_series(1, indnatts) AS attpos \
                       FROM ( \
                         SELECT ci.relname AS idxname, ci.reltuples, ci.relpages, i.indrelid AS tbloid, i.indexrelid AS idxoid, \
                                coalesce(substring(array_to_string(ci.reloptions, ' ') FROM 'fillfactor=([0-9]+)')::smallint, 90) AS fillfactor, \
                                i.indnatts, pg_catalog.string_to_array(pg_catalog.textin(pg_catalog.int2vectorout(i.indkey)), ' ')::int[] AS indkey \
                         FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ci ON ci.oid = i.indexrelid \
                         WHERE ci.relam = (SELECT oid FROM pg_catalog.pg_am WHERE amname = 'btree') AND ci.relpages > 0 \
                       ) AS idx_data \
                     ) AS ic \
                     JOIN pg_catalog.pg_class ct ON ct.oid = ic.tbloid \
                     LEFT JOIN pg_catalog.pg_attribute a1 ON ic.indkey[ic.attpos] <> 0 AND a1.attrelid = ic.tbloid AND a1.attnum = ic.indkey[ic.attpos] \
                     LEFT JOIN pg_catalog.pg_attribute a2 ON ic.indkey[ic.attpos] = 0 AND a2.attrelid = ic.idxoid AND a2.attnum = ic.attpos \
                   ) i \
                   JOIN pg_catalog.pg_namespace n ON n.oid = i.relnamespace \
                   JOIN pg_catalog.pg_stats s ON s.schemaname = n.nspname AND s.tablename = i.attrelname AND s.attname = i.attname \
                   WHERE n.nspname = {scoped} \
                   GROUP BY 1, 2, 3, 4, 5, 6, 7, 8 \
                 ) AS rows_data_stats \
               ) AS rows_hdr_pdg_stats \
             ) AS relation_stats \
             ) AS bloat ORDER BY wasted_bytes DESC, name"
    )
}

pub fn bloat(call: Call, args: BloatArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        if args.exact_table.trim().is_empty() {
            let sql = bloat_sql(&call.settings().schema.value);
            return run_catalog(&call, "bloat", &sql, args.row_cap).await;
        }
        let table = super::ddl::scoped_name(&call, "exact_table", &args.exact_table)?;
        let extension = extension_schema(&call, "pgstattuple").await?;
        let sql = exact_bloat_sql(&extension, &table);
        run_catalog(&call, "bloat", &sql, args.row_cap).await
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingsArgs {
    #[serde(default)]
    #[schemars(description = "LIKE pattern on the setting name; empty means every setting.")]
    pub pattern: String,
    #[serde(default)]
    #[schemars(description = "Keep only settings whose value differs from the default.")]
    pub changed_only: bool,
    #[serde(default)]
    #[schemars(
        description = "Rows per page. 0 uses the configured default (100). The maximum is 1000."
    )]
    pub row_cap: u32,
}

fn settings_sql(args: &SettingsArgs) -> String {
    let mut filters = Vec::new();
    if !args.pattern.trim().is_empty() {
        filters.push(format!("name LIKE {}", quote_literal(args.pattern.trim())));
    }
    if args.changed_only {
        filters.push("setting IS DISTINCT FROM boot_val".to_owned());
    }
    let where_clause = if filters.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", filters.join(" AND "))
    };
    format!(
        "SELECT name, setting, unit, source, pending_restart, category, short_desc \
             FROM pg_catalog.pg_settings{where_clause} ORDER BY name"
    )
}

pub fn settings(call: Call, args: SettingsArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let sql = settings_sql(&args);
        run_catalog(&call, "settings", &sql, args.row_cap).await
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TopQueriesOrder {
    #[default]
    TotalTime,
    MeanTime,
    Calls,
    Rows,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TopQueriesArgs {
    #[serde(default)]
    pub order_by: TopQueriesOrder,
    #[serde(default)]
    #[schemars(
        description = "Rows per page. 0 uses the configured default (100). The maximum is 1000."
    )]
    pub row_cap: u32,
}

pub fn top_queries(call: Call, args: TopQueriesArgs) -> BoxFuture<'static, Outcome> {
    Box::pin(async move {
        let (extension_schema, installed) =
            installed_extension(&call, "pg_stat_statements").await?;
        if !version_at_least(&installed, (1, 8)) {
            return Err(Error::ExtensionOutdated {
                name: "pg_stat_statements".to_owned(),
                installed,
                needed: STAT_STATEMENTS_NEEDED.to_owned(),
            }
            .into());
        }
        let order = match args.order_by {
            TopQueriesOrder::TotalTime => "total_exec_time DESC",
            TopQueriesOrder::MeanTime => "mean_exec_time DESC",
            TopQueriesOrder::Calls => "calls DESC",
            TopQueriesOrder::Rows => "rows DESC",
        };
        let sql = format!(
            "SELECT queryid::text, calls, round(total_exec_time::numeric, 2) AS total_exec_time_ms, \
             round(mean_exec_time::numeric, 3) AS mean_exec_time_ms, rows, shared_blks_hit, shared_blks_read, \
             left(query, 500) AS query \
             FROM {}.pg_stat_statements WHERE dbid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database()) \
             ORDER BY {order}, queryid",
            crate::render::quote_ident(&extension_schema)
        );
        let mut classification = verify(&sql, &["SelectStmt"])?;
        classification.relations.clear();
        let mut facts = facts_for(&classification);
        facts.operation = Some("top_queries".to_owned());
        let result = call
            .engine()
            .run_read(&sql, call.caps(args.row_cap))
            .await
            .map_err(|error| ToolFailure::from(error).with_facts(facts.clone()))?;
        let facts = facts.with_result(&result);
        let text = result.render_text();
        Ok(ToolOutput::structured(&result, text)?
            .with_facts(facts)
            .into())
    })
}

pub fn routes() -> Result<Vec<Route>> {
    Ok(vec![
        route::<ActivityArgs, ResultSet, _>(
            &tool_specs::PG_ACTIVITY,
            ACTIVITY_DESCRIPTION,
            activity,
        )?,
        route::<LocksArgs, ResultSet, _>(&tool_specs::PG_LOCKS, LOCKS_DESCRIPTION, locks)?,
        route::<super::health::NoArgs, ResultSet, _>(
            &tool_specs::PG_REPLICATION,
            REPLICATION_DESCRIPTION,
            replication,
        )?,
        route::<super::health::NoArgs, ResultSet, _>(&tool_specs::PG_WAL, WAL_DESCRIPTION, wal)?,
        route::<super::health::NoArgs, ResultSet, _>(
            &tool_specs::PG_INDEXES_HEALTH,
            INDEXES_HEALTH_DESCRIPTION,
            indexes_health,
        )?,
        route::<BloatArgs, ResultSet, _>(&tool_specs::PG_BLOAT, BLOAT_DESCRIPTION, bloat)?,
        route::<SettingsArgs, ResultSet, _>(
            &tool_specs::PG_SETTINGS,
            SETTINGS_DESCRIPTION,
            settings,
        )?,
        route::<TopQueriesArgs, ResultSet, _>(
            &tool_specs::PG_TOP_QUERIES,
            TOP_QUERIES_DESCRIPTION,
            top_queries,
        )?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_versions_compare_as_major_minor_pairs() {
        assert!(version_at_least("1.8", (1, 8)));
        assert!(version_at_least("1.10", (1, 8)));
        assert!(version_at_least("2.0", (1, 8)));
        assert!(!version_at_least("1.7", (1, 8)));
        assert!(!version_at_least("garbage", (1, 8)));
        let error = Error::ExtensionOutdated {
            name: "pg_stat_statements".to_owned(),
            installed: "1.7".to_owned(),
            needed: "1.8".to_owned(),
        };
        assert_eq!(error.id(), crate::error::ErrorId::ExtensionOutdated);
        assert!(
            error
                .remedy()
                .contains("ALTER EXTENSION pg_stat_statements UPDATE")
        );
    }

    #[test]
    fn every_monitoring_statement_parses_as_one_select() {
        let activity = activity_sql(&ActivityArgs {
            include_idle: false,
            min_duration_seconds: 5,
            row_cap: 0,
        });
        assert!(activity.contains(">= 5"));
        let changed = settings_sql(&SettingsArgs {
            pattern: "work_%".to_owned(),
            changed_only: true,
            row_cap: 0,
        });
        assert!(changed.contains("name LIKE 'work_%'"));
        let old = wal_sql(crate::engine::Features::from_version(150_000));
        let new = wal_sql(crate::engine::Features::from_version(180_000));
        assert!(old.contains("pg_stat_bgwriter"));
        assert!(new.contains("pg_stat_checkpointer"));
        assert!(new.contains("pg_stat_io"));
        for statement in [
            activity,
            changed,
            old,
            new,
            LOCKS_SQL.to_owned(),
            REPLICATION_SQL.to_owned(),
            indexes_health_sql("app"),
            bloat_sql("app"),
        ] {
            let parsed = verify(&statement, &["SelectStmt"]).unwrap();
            assert!(parsed.refusals.is_empty(), "{statement}");
        }
    }
}
