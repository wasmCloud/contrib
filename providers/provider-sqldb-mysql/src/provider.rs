#![cfg(not(doctest))]

//! SQL-powered database access provider implementing `wasmcloud:mysql` for connecting
//! to MySQL servers.
//!
//! This implementation is multi-threaded and operations between different actors
//! use different connections and can run in parallel.
//!

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use mysql_async::{prelude::*, Pool};
use sha2::{Digest as _, Sha256};
use tokio::sync::RwLock;
use tracing::{error, instrument, warn};
use ulid::Ulid;

use wasmcloud_provider_sdk::{
    get_connection, propagate_trace_for_ctx, run_provider, LinkConfig, LinkDeleteInfo, Provider,
};
use wasmcloud_provider_sdk::{initialize_observability, serve_provider_exports};

use crate::bindings::{
    PreparedStatementExecError, PreparedStatementToken, QueryError, ResultRow,
    StatementPrepareError,
};

use crate::bindings::wasmcloud::mysql::types::MysqlValue;

use crate::config::{extract_prefixed_conn_config, ConnectionCreateOptions};

use wasmcloud_provider_sdk::Context;

/// Whether to share connections by URL
///
/// This option indicates that URLs with identical connection configurations will be shared/reused by
/// components that are linked with the same configurations
const CONFIG_SHARE_CONNECTIONS_BY_URL_KEY: &str = "MYSQL_SHARE_CONNECTIONS_BY_URL";

/// A unique identifier for a created connection
type SourceId = String;

/// A query used in the process of creating a prepared statement
type PreparedStatementQuery = String;

/// Parameters determined to be used in a statement
///
/// This value is constructed after running a prepare against a given
/// client from a given pool, and saving the relevant MySQL column type information.
type StatementParams = Vec<mysql_async::consts::ColumnType>;

/// Information about a given prepared statement
type PreparedStatementInfo = (PreparedStatementQuery, StatementParams, SourceId);

/// Shared connection keys are keys that identify shared connections
///
/// This is the hash of the connection configuration, to avoid printing credentials inadvertently.
type SharedConnectionKey = String;

/// Type of MySQL connection - either direct or shared
#[derive(Clone)]
enum MysqlConnection {
    /// Direct connection to a pool
    Direct(Pool),
    /// Shared connection, identified by the hash of the connection configuration
    Shared(String),
}

#[derive(Clone, Default)]
pub struct MysqlProvider {
    /// Database connections indexed by source ID name
    connections: Arc<RwLock<HashMap<SourceId, MysqlConnection>>>,
    /// Shared connection pools indexed by configuration hash
    shared_connections: Arc<RwLock<HashMap<SharedConnectionKey, Pool>>>,
    /// Lookup of prepared statements to the statement and the source ID that prepared them
    prepared_statements: Arc<RwLock<HashMap<PreparedStatementToken, PreparedStatementInfo>>>,
}

impl MysqlProvider {
    fn name() -> &'static str {
        "sqldb-mysql-provider"
    }

    /// Generate a connection string from ConnectionCreateOptions for hashing
    fn connection_string_for_hashing(opts: &ConnectionCreateOptions) -> String {
        format!(
            "mysql://{}:{}@{}:{}/{}?tls_required={}&pool_size={:?}",
            opts.username,
            opts.password,
            opts.host,
            opts.port,
            opts.database,
            opts.tls_required,
            opts.pool_size
        )
    }

    /// Get a pool for the given source_id, resolving shared connections if necessary
    async fn get_pool(&self, source_id: &str) -> Result<Pool, String> {
        let connections = self.connections.read().await;
        let connection = connections
            .get(source_id)
            .ok_or_else(|| format!("missing connection pool for source [{source_id}]"))?;

        match connection {
            MysqlConnection::Direct(pool) => Ok(pool.clone()),
            MysqlConnection::Shared(key) => {
                let shared = self.shared_connections.read().await;
                shared
                    .get(key)
                    .cloned()
                    .ok_or_else(|| format!("no shared connection found with key [{key}]"))
            }
        }
    }

    /// Run [`MysqlProvider`] as a wasmCloud provider
    pub async fn run() -> anyhow::Result<()> {
        initialize_observability!(
            MysqlProvider::name(),
            std::env::var_os("PROVIDER_SQLDB_MYSQL_FLAMEGRAPH_PATH")
        );
        let provider = MysqlProvider::default();
        let shutdown = run_provider(provider.clone(), MysqlProvider::name())
            .await
            .context("failed to run provider")?;
        let connection = get_connection();
        let wrpc = connection
            .get_wrpc_client(connection.provider_key())
            .await?;
        serve_provider_exports(&wrpc, provider, shutdown, crate::bindings::serve)
            .await
            .context("failed to serve provider exports")
    }

    /// Create and store a connection pool, if not already present
    async fn ensure_pool(
        &self,
        source_id: &str,
        create_opts: ConnectionCreateOptions,
        share_connections: bool,
    ) -> Result<()> {
        // If sharing is enabled, check if we already have a shared connection for this configuration
        if share_connections {
            let connection_string = Self::connection_string_for_hashing(&create_opts);
            let shared_key = format!("{:X}", Sha256::digest(&connection_string));

            // Check if we already have this shared connection
            {
                let shared_connections = self.shared_connections.read().await;
                if shared_connections.contains_key(&shared_key) {
                    let mut connections = self.connections.write().await;
                    connections.insert(source_id.into(), MysqlConnection::Shared(shared_key));
                    return Ok(());
                }
            }
        }

        // Exit early if a pool with the given source ID is already present
        {
            let connections = self.connections.read().await;
            if connections.get(source_id).is_some() {
                return Ok(());
            }
        }

        // Build the new connection pool
        let opts = mysql_async::Opts::from(create_opts.clone());
        let pool = Pool::new(opts);

        if share_connections {
            // Store as shared connection
            let connection_string = Self::connection_string_for_hashing(&create_opts);
            let shared_key = format!("{:X}", Sha256::digest(&connection_string));

            // Store the shared connection first, then reference it
            let mut shared_connections = self.shared_connections.write().await;
            shared_connections.insert(shared_key.clone(), pool);
            drop(shared_connections);

            let mut connections = self.connections.write().await;
            connections.insert(source_id.into(), MysqlConnection::Shared(shared_key));
        } else {
            // Store as direct connection
            let mut connections = self.connections.write().await;
            connections.insert(source_id.into(), MysqlConnection::Direct(pool));
        }

        Ok(())
    }

    /// Perform a query
    async fn do_query(
        &self,
        source_id: &str,
        query: &str,
        params: Vec<MysqlValue>,
    ) -> Result<Vec<ResultRow>, QueryError> {
        // Validate parameters before executing
        for param in &params {
            match param {
                MysqlValue::Json(json_str) => {
                    crate::bindings::validate_json_string(json_str).map_err(|e| {
                        QueryError::InvalidParams(format!("Invalid JSON parameter: {e}"))
                    })?;
                }
                MysqlValue::Geometry(wkb)
                | MysqlValue::PointGeom(wkb)
                | MysqlValue::Linestring(wkb)
                | MysqlValue::Polygon(wkb)
                | MysqlValue::Multipoint(wkb)
                | MysqlValue::Multilinestring(wkb)
                | MysqlValue::Multipolygon(wkb)
                | MysqlValue::Geometrycollection(wkb) => {
                    if !crate::bindings::is_valid_wkb(wkb) {
                        return Err(QueryError::InvalidParams(
                            "Invalid WKB geometry data".to_string(),
                        ));
                    }
                }
                _ => {}
            }
        }

        let pool = self.get_pool(source_id).await.map_err(|e| {
            QueryError::Unexpected(format!(
                "missing connection pool for source [{source_id}] while querying: {e}"
            ))
        })?;

        let mut conn = pool.get_conn().await.map_err(|e| {
            QueryError::Unexpected(format!("failed to get connection from pool: {e}"))
        })?;

        // Convert MysqlValue to mysql_async::Value using the bindings conversion
        let mysql_params: Vec<mysql_async::Value> = params.into_iter().map(|v| v.into()).collect();

        let rows: Vec<mysql_async::Row> = conn
            .exec(query, mysql_params)
            .await
            .map_err(|e| crate::bindings::mysql_error_to_query_error(e))?;

        // Convert MySQL rows to ResultRow
        let result_rows: Result<Vec<ResultRow>, _> = rows
            .into_iter()
            .map(|row| crate::bindings::into_result_row(row))
            .collect();

        result_rows.map_err(|e| QueryError::Unexpected(format!("failed to convert rows: {e}")))
    }

    /// Perform a batch query (multiple SQL statements)
    async fn do_query_batch(&self, source_id: &str, query: &str) -> Result<(), QueryError> {
        let pool = self.get_pool(source_id).await.map_err(|e| {
            QueryError::Unexpected(format!(
                "missing connection pool for source [{source_id}] while querying: {e}"
            ))
        })?;

        let mut conn = pool.get_conn().await.map_err(|e| {
            QueryError::Unexpected(format!("failed to get connection from pool: {e}"))
        })?;

        conn.query_drop(query)
            .await
            .map_err(|e| crate::bindings::mysql_error_to_query_error(e))?;

        Ok(())
    }

    /// Prepare a statement
    async fn do_statement_prepare(
        &self,
        source_id: &str,
        query: &str,
    ) -> Result<PreparedStatementToken, StatementPrepareError> {
        if query.trim().is_empty() {
            return Err(StatementPrepareError::Unexpected(
                "Query cannot be empty".to_string(),
            ));
        }

        // Get a connection to actually prepare the statement and validate it
        let pool = self.get_pool(source_id).await.map_err(|e| {
            StatementPrepareError::Unexpected(format!(
                "failed to find connection pool for source [{source_id}]: {e}"
            ))
        })?;

        let mut conn = pool.get_conn().await.map_err(|e| {
            StatementPrepareError::Unexpected(format!("failed to get connection from pool: {e}"))
        })?;

        let statement = conn
            .prep(query)
            .await
            .map_err(|e| crate::bindings::mysql_error_to_statement_prepare_error(e))?;

        let param_types: Vec<mysql_async::consts::ColumnType> = statement
            .params()
            .iter()
            .map(|param| param.column_type())
            .collect();

        let statement_token = format!("prepared-statement-{}", Ulid::new().to_string());

        let mut prepared_statements = self.prepared_statements.write().await;
        prepared_statements.insert(
            statement_token.clone(),
            (query.into(), param_types, source_id.into()),
        );

        Ok(statement_token)
    }

    /// Execute a prepared statement, returning the number of rows affected
    async fn do_statement_execute(
        &self,
        statement_token: &str,
        params: Vec<MysqlValue>,
    ) -> Result<u64, PreparedStatementExecError> {
        // Validate parameters before executing
        for param in &params {
            match param {
                MysqlValue::Json(json_str) => {
                    crate::bindings::validate_json_string(json_str).map_err(|e| {
                        PreparedStatementExecError::QueryError(QueryError::InvalidParams(format!(
                            "Invalid JSON parameter: {e}"
                        )))
                    })?;
                }
                MysqlValue::Geometry(wkb)
                | MysqlValue::PointGeom(wkb)
                | MysqlValue::Linestring(wkb)
                | MysqlValue::Polygon(wkb)
                | MysqlValue::Multipoint(wkb)
                | MysqlValue::Multilinestring(wkb)
                | MysqlValue::Multipolygon(wkb)
                | MysqlValue::Geometrycollection(wkb) => {
                    if !crate::bindings::is_valid_wkb(wkb) {
                        return Err(PreparedStatementExecError::QueryError(
                            QueryError::InvalidParams("Invalid WKB geometry data".to_string()),
                        ));
                    }
                }
                _ => {}
            }
        }

        let statements = self.prepared_statements.read().await;
        let (query, param_types, source_id) = statements.get(statement_token).ok_or_else(|| {
            PreparedStatementExecError::Unexpected(format!(
                "missing prepared statement with statement ID [{statement_token}]"
            ))
        })?;

        let pool = self.get_pool(source_id).await.map_err(|e| {
            PreparedStatementExecError::Unexpected(format!(
                "missing connection pool for token [{source_id}], statement ID [{statement_token}]: {e}"
            ))
        })?;

        let mut conn = pool.get_conn().await.map_err(|e| {
            PreparedStatementExecError::Unexpected(format!(
                "failed to get connection from pool: {e}"
            ))
        })?;

        // Validate parameter types against expected types
        if params.len() != param_types.len() {
            return Err(PreparedStatementExecError::QueryError(
                QueryError::InvalidParams(format!(
                    "Parameter count mismatch: expected {}, got {}",
                    param_types.len(),
                    params.len()
                )),
            ));
        }

        // Validate each parameter type
        for (i, (param, expected_type)) in params.iter().zip(param_types.iter()).enumerate() {
            if let Err(e) =
                crate::bindings::validate_mysql_value_for_column_type(param, *expected_type)
            {
                return Err(PreparedStatementExecError::QueryError(
                    QueryError::InvalidParams(format!(
                        "Parameter {} type validation failed: {}",
                        i + 1,
                        e
                    )),
                ));
            }
        }

        // Convert MysqlValue to mysql_async::Value using the bindings conversion
        let mysql_params: Vec<mysql_async::Value> = params.into_iter().map(|v| v.into()).collect();

        // Execute the prepared statement - mysql_async will prepare it automatically if needed
        let _result: Vec<mysql_async::Row> = conn
            .exec(query, mysql_params)
            .await
            .map_err(|e| crate::bindings::mysql_error_to_prepared_error(e))?;

        // Get affected rows from the connection's info
        // For INSERT/UPDATE/DELETE operations, we can check affected_rows
        let affected_rows = conn.affected_rows();

        Ok(affected_rows)
    }
}

impl Provider for MysqlProvider {
    /// Handle being linked to a source (likely a component) as a target
    ///
    /// Components are expected to provide references to named configuration via link definitions
    /// which contain keys named `MYSQL_*` detailing configuration for connecting to MySQL.
    #[instrument(level = "debug", skip_all, fields(source_id))]
    async fn receive_link_config_as_target(
        &self,
        link_config @ LinkConfig { source_id, .. }: LinkConfig<'_>,
    ) -> anyhow::Result<()> {
        // Attempt to parse a configuration from the map with the prefix MYSQL_
        let Some(db_cfg) = extract_prefixed_conn_config("MYSQL_", &link_config) else {
            // If we failed to find a config on the link, then we
            warn!(source_id, "no link-level DB configuration");
            return Ok(());
        };

        // Check if connection sharing is enabled
        let share_connections = if let Some(value) =
            link_config.config.get(CONFIG_SHARE_CONNECTIONS_BY_URL_KEY)
        {
            matches!(value.to_lowercase().as_str(), "true" | "yes")
        } else if let Some(secret) = link_config.secrets.get(CONFIG_SHARE_CONNECTIONS_BY_URL_KEY) {
            if let Some(value) = secret.as_string() {
                matches!(value.to_lowercase().as_str(), "true" | "yes")
            } else {
                false
            }
        } else {
            false
        };

        // Create a pool if one isn't already present for this particular source
        if let Err(error) = self.ensure_pool(source_id, db_cfg, share_connections).await {
            error!(?error, source_id, "failed to create connection");
        };

        Ok(())
    }

    /// Handle notification that a link is dropped
    ///
    /// Generally we can release the resources (connections) associated with the source
    #[instrument(level = "info", skip_all, fields(source_id = info.get_source_id()))]
    async fn delete_link_as_target(&self, info: impl LinkDeleteInfo) -> anyhow::Result<()> {
        let source_id = info.get_source_id();
        let mut prepared_statements = self.prepared_statements.write().await;
        prepared_statements
            .retain(|_stmt_token, (_query, _param_types, src_id)| src_id != source_id);
        drop(prepared_statements);
        let mut connections = self.connections.write().await;
        connections.remove(source_id);
        drop(connections);
        Ok(())
    }

    /// Handle shutdown request by closing all connections
    #[instrument(level = "debug", skip_all)]
    async fn shutdown(&self) -> anyhow::Result<()> {
        let mut prepared_statements = self.prepared_statements.write().await;
        prepared_statements.drain();
        let mut connections = self.connections.write().await;
        connections.drain();
        Ok(())
    }
}

/// Implement the `wasmcloud:mysql/query` interface for [`MysqlProvider`]
impl crate::bindings::query::Handler<Option<Context>> for MysqlProvider {
    #[instrument(level = "debug", skip_all, fields(query))]
    async fn query(
        &self,
        ctx: Option<Context>,
        query: String,
        params: Vec<MysqlValue>,
    ) -> Result<Result<Vec<ResultRow>, QueryError>> {
        propagate_trace_for_ctx!(ctx);
        let Some(Context {
            component: Some(source_id),
            ..
        }) = ctx
        else {
            return Ok(Err(QueryError::Unexpected(
                "unexpectedly missing source ID".into(),
            )));
        };

        Ok(self.do_query(&source_id, &query, params).await)
    }

    #[instrument(level = "debug", skip_all, fields(query))]
    async fn query_batch(
        &self,
        ctx: Option<Context>,
        query: String,
    ) -> Result<Result<(), QueryError>> {
        propagate_trace_for_ctx!(ctx);
        let Some(Context {
            component: Some(source_id),
            ..
        }) = ctx
        else {
            return Ok(Err(QueryError::Unexpected(
                "unexpectedly missing source ID".into(),
            )));
        };

        Ok(self.do_query_batch(&source_id, &query).await)
    }
}

/// Implement the `wasmcloud:mysql/prepared` interface for [`MysqlProvider`]
impl crate::bindings::prepared::Handler<Option<Context>> for MysqlProvider {
    #[instrument(level = "debug", skip_all, fields(query))]
    async fn prepare(
        &self,
        ctx: Option<Context>,
        query: String,
    ) -> Result<Result<PreparedStatementToken, StatementPrepareError>> {
        propagate_trace_for_ctx!(ctx);
        let Some(Context {
            component: Some(source_id),
            ..
        }) = ctx
        else {
            return Ok(Err(StatementPrepareError::Unexpected(
                "unexpectedly missing source ID".into(),
            )));
        };
        Ok(self.do_statement_prepare(&source_id, &query).await)
    }

    #[instrument(level = "debug", skip_all, fields(statement_token))]
    async fn exec(
        &self,
        ctx: Option<Context>,
        statement_token: PreparedStatementToken,
        params: Vec<MysqlValue>,
    ) -> Result<Result<u64, PreparedStatementExecError>> {
        propagate_trace_for_ctx!(ctx);
        Ok(self.do_statement_execute(&statement_token, params).await)
    }
}
