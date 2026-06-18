use mysql_async::{Opts, OptsBuilder, PoolConstraints, PoolOpts, SslOpts};
use tracing::warn;
use wasmcloud_provider_sdk::{core::secrets::SecretValue, LinkConfig};

const MYSQL_DEFAULT_PORT: u16 = 3306;

/// Creation options for a MySQL connection
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectionCreateOptions {
    /// Hostname of the MySQL server to connect to
    pub host: String,
    /// Port on which to connect to the MySQL server
    pub port: u16,
    /// Username used when accessing the MySQL server
    pub username: String,
    /// Password used when accessing the MySQL server
    pub password: String,
    /// Database to connect to
    pub database: String,
    /// Whether TLS is required for the connection
    pub tls_required: bool,
    /// Optional connection pool size
    pub pool_size: Option<usize>,
}

impl From<ConnectionCreateOptions> for Opts {
    fn from(opts: ConnectionCreateOptions) -> Self {
        let pool_opts = match opts.pool_size {
            Some(size) => PoolConstraints::new(0, size)
                .map(|constraints| PoolOpts::new().with_constraints(constraints))
                .unwrap_or_else(|| PoolOpts::new()),
            None => PoolOpts::new(),
        };

        let mut builder = OptsBuilder::default()
            .ip_or_hostname(opts.host)
            .tcp_port(opts.port)
            .user(Some(opts.username))
            .pass(Some(opts.password))
            .db_name(Some(opts.database))
            .pool_opts(pool_opts);

        if opts.tls_required {
            builder = builder.ssl_opts(SslOpts::default());
        }

        builder.into()
    }
}

/// Parse the options for MySQL configuration from a [`HashMap`], with a given prefix to the keys
///
/// For example given a prefix like `EXAMPLE_`, and a HashMap that contains an entry like ("EXAMPLE_HOST", "localhost"),
/// the parsed [`ConnectionCreateOptions`] would contain "localhost" as the host.
pub(crate) fn extract_prefixed_conn_config(
    prefix: &str,
    link_config: &LinkConfig,
) -> Option<ConnectionCreateOptions> {
    let LinkConfig {
        config, secrets, ..
    } = link_config;

    let keys = [
        format!("{prefix}HOST"),
        format!("{prefix}PORT"),
        format!("{prefix}USERNAME"),
        format!("{prefix}PASSWORD"),
        format!("{prefix}DATABASE"),
        format!("{prefix}TLS_REQUIRED"),
        format!("{prefix}POOL_SIZE"),
    ];

    match keys
        .iter()
        .map(|k| {
            // Prefer fetching from secrets, but fall back to config if not found
            match (secrets.get(k).and_then(SecretValue::as_string), config.get(k)) {
                (Some(s), Some(_)) => {
                    warn!("secret value [{k}] was found in secrets, but also exists in config. The value in secrets will be used.");
                    Some(s)
                }
                (Some(s), _) => Some(s),
                // Offer a warning for the password, but other values are fine to be in config
                (None, Some(c)) if k == &format!("{prefix}PASSWORD") => {
                    warn!("secret value [{k}] was not found in secrets, but exists in config. Prefer using secrets for sensitive values.");
                    Some(c.as_str())
                }
                (None, Some(c)) => {
                    Some(c.as_str())
                }
                (_, None) => None,
            }
        })
        .collect::<Vec<Option<&str>>>()[..]
    {
        [Some(host), Some(port), Some(username), Some(password), Some(database), tls_required, pool_size] =>
        {
            let pool_size = pool_size.and_then(|pool_size| {
                pool_size.parse::<usize>().ok().or_else(|| {
                    warn!("invalid pool size value [{pool_size}], using default");
                    None
                })
            });

            Some(ConnectionCreateOptions {
                host: host.to_string(),
                port: port.parse::<u16>().unwrap_or_else(|_e| {
                    warn!("invalid port value [{port}], using {MYSQL_DEFAULT_PORT}");
                    MYSQL_DEFAULT_PORT
                }),
                username: username.to_string(),
                password: password.to_string(),
                tls_required: tls_required.is_some_and(|tls_required| {
                    matches!(tls_required.to_lowercase().as_str(), "true" | "yes")
                }),
                database: database.to_string(),
                pool_size,
            })
        }
        _ => {
            warn!("failed to find required keys in configuration: [{:?}]", keys);
            None
        }
    }
}
