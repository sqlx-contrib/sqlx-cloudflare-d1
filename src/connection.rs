use std::fmt::{self, Debug, Formatter};
use std::future::{self, Future};
use std::str::FromStr;
use std::time::Duration;

use log::LevelFilter;
use sqlx_core::connection::{ConnectOptions, Connection};
use sqlx_core::error::Error;
use sqlx_core::transaction::Transaction;
use sqlx_core::Url;

use crate::D1;

/// A connection to a D1 database: a Workers D1 binding.
///
/// There is no socket behind it and nothing to pool. Build one per request
/// from the binding, with [`D1Connection::from_env`] or [`D1Connection::new`].
///
/// Both `&mut D1Connection` and `&D1Connection` are sqlx executors. The
/// binding is stateless, so a shared reference is enough to run a query, and
/// one connection can serve concurrent queries within a request.
pub struct D1Connection {
    // `worker::D1Database` is already `Send + Sync` (`worker` implements both,
    // on the grounds that Workers are single-threaded), so it needs no wrapper
    // of ours -- only the JavaScript futures it produces do.
    pub(crate) db: worker::D1Database,
}

impl D1Connection {
    /// Wraps a D1 binding.
    #[must_use]
    pub fn new(db: worker::D1Database) -> Self {
        Self { db }
    }

    /// Looks up the D1 binding called `binding` in the Worker's environment.
    ///
    /// # Errors
    ///
    /// When there is no binding by that name, or it is not a D1 database.
    pub fn from_env(env: &worker::Env, binding: &str) -> worker::Result<Self> {
        env.d1(binding).map(Self::new)
    }

    /// The binding this connection wraps, for anything this crate does not
    /// cover -- `dump`, `exec`, sessions.
    #[must_use]
    pub fn into_inner(self) -> worker::D1Database {
        self.db
    }
}

impl Debug for D1Connection {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("D1Connection").finish_non_exhaustive()
    }
}

impl Connection for D1Connection {
    type Database = D1;

    type Options = D1ConnectOptions;

    // Nothing to close: the binding belongs to the Worker, not to us.
    fn close(self) -> impl Future<Output = Result<(), Error>> + Send + 'static {
        future::ready(Ok(()))
    }

    fn close_hard(self) -> impl Future<Output = Result<(), Error>> + Send + 'static {
        future::ready(Ok(()))
    }

    // A binding cannot go stale the way a socket can, so there is nothing a
    // round trip would tell us that the next query won't.
    fn ping(&mut self) -> impl Future<Output = Result<(), Error>> + Send + '_ {
        future::ready(Ok(()))
    }

    /// Always fails -- see [`D1TransactionManager`](crate::D1TransactionManager).
    fn begin(&mut self) -> impl Future<Output = Result<Transaction<'_, D1>, Error>> + Send + '_ {
        Transaction::begin(self, None)
    }

    fn shrink_buffers(&mut self) {}

    fn flush(&mut self) -> impl Future<Output = Result<(), Error>> + Send + '_ {
        future::ready(Ok(()))
    }

    fn should_flush(&self) -> bool {
        false
    }
}

/// The options type sqlx requires of every connection.
///
/// It parses `d1://<binding>` so the type is constructible, but
/// [`connect`](ConnectOptions::connect) always fails: a D1 connection comes
/// from a Workers binding, which only the Worker's `Env` can hand out. Use
/// [`D1Connection::from_env`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D1ConnectOptions {
    binding: String,
}

impl D1ConnectOptions {
    /// The binding name from the URL.
    #[must_use]
    pub fn binding(&self) -> &str {
        &self.binding
    }
}

impl FromStr for D1ConnectOptions {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let url = Url::parse(s).map_err(|error| Error::Configuration(error.into()))?;
        Self::from_url(&url)
    }
}

impl ConnectOptions for D1ConnectOptions {
    type Connection = D1Connection;

    fn from_url(url: &Url) -> Result<Self, Error> {
        if url.scheme() != "d1" {
            return Err(Error::Configuration(
                format!(
                    "expected a `d1://<binding>` URL, got scheme `{}`",
                    url.scheme()
                )
                .into(),
            ));
        }

        match url.host_str() {
            Some(binding) if !binding.is_empty() => Ok(Self {
                binding: binding.to_owned(),
            }),
            _ => Err(Error::Configuration(
                "expected a `d1://<binding>` URL with a binding name".into(),
            )),
        }
    }

    fn to_url_lossy(&self) -> Url {
        Url::parse(&format!("d1://{}", self.binding)).expect("a binding name parsed from a URL")
    }

    fn connect(&self) -> impl Future<Output = Result<D1Connection, Error>> + Send + '_ {
        future::ready(Err(Error::Configuration(
            format!(
                "a D1 connection cannot be opened from a URL; get the `{}` binding from the \
                 Worker's environment with `D1Connection::from_env`",
                self.binding
            )
            .into(),
        )))
    }

    // No statement logging yet, so there is nothing for these to configure.
    fn log_statements(self, _level: LevelFilter) -> Self {
        self
    }

    fn log_slow_statements(self, _level: LevelFilter, _duration: Duration) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use sqlx_core::connection::ConnectOptions;

    use super::D1ConnectOptions;

    #[test]
    fn parses_a_binding_url() {
        let options: D1ConnectOptions = "d1://DB".parse().unwrap();

        // `Url` lowercases a special scheme's host but leaves an unknown
        // scheme's alone, so a binding keeps its case.
        assert_eq!(options.binding(), "DB");
        assert_eq!(options.to_url_lossy().as_str(), "d1://DB");
    }

    #[test]
    fn rejects_other_schemes_and_a_missing_binding() {
        assert!("sqlite://DB".parse::<D1ConnectOptions>().is_err());
        assert!("d1://".parse::<D1ConnectOptions>().is_err());
    }
}
