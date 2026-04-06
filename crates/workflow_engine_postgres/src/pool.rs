// Copyright 2026 Desicool
//
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use native_tls::TlsConnector;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_native_tls::{TlsConnector as TokioTlsConnector, TlsStream as TokioNativeTlsStream};
use tokio_postgres::config::SslMode;
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect, TlsStream};
use tokio_postgres::NoTls;

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Build a deadpool-postgres connection pool from a database URL.
///
/// TLS is enabled automatically when the URL does not explicitly set
/// `sslmode=disable`. Certificate verification can be enabled by setting
/// the `DB_SSL_VERIFY_CERT` environment variable to `1`, `true`, `yes`, or `on`.
pub fn build_pool(database_url: &str) -> Result<Pool> {
    let pg_config: tokio_postgres::Config = database_url.parse()?;

    let mgr_config = ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    };

    if uses_tls(&pg_config) {
        let connector = build_tls_connector()?;
        let mgr = Manager::from_config(pg_config, NativeTlsConnector::new(connector), mgr_config);
        Ok(Pool::builder(mgr).max_size(16).build()?)
    } else {
        let mgr = Manager::from_config(pg_config, NoTls, mgr_config);
        Ok(Pool::builder(mgr).max_size(16).build()?)
    }
}

fn uses_tls(pg_config: &tokio_postgres::Config) -> bool {
    pg_config.get_ssl_mode() != SslMode::Disable
}

fn build_tls_connector() -> Result<TlsConnector> {
    let mut builder = TlsConnector::builder();
    if !tls_verification_enabled() {
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
    }
    Ok(builder.build()?)
}

fn tls_verification_enabled() -> bool {
    matches!(
        std::env::var("DB_SSL_VERIFY_CERT")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

// ── TLS plumbing ──────────────────────────────────────────────────────────────

#[derive(Clone)]
struct NativeTlsConnector {
    connector: TlsConnector,
}

impl NativeTlsConnector {
    fn new(connector: TlsConnector) -> Self {
        Self { connector }
    }
}

impl<S> MakeTlsConnect<S> for NativeTlsConnector
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = NativeTlsStream<S>;
    type TlsConnect = NativeTlsConnect;
    type Error = native_tls::Error;

    fn make_tls_connect(&mut self, domain: &str) -> Result<Self::TlsConnect, Self::Error> {
        Ok(NativeTlsConnect {
            connector: TokioTlsConnector::from(self.connector.clone()),
            domain: domain.to_string(),
        })
    }
}

struct NativeTlsConnect {
    connector: TokioTlsConnector,
    domain: String,
}

impl<S> TlsConnect<S> for NativeTlsConnect
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = NativeTlsStream<S>;
    type Error = native_tls::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Stream, Self::Error>> + Send>>;

    fn connect(self, stream: S) -> Self::Future {
        Box::pin(async move {
            let stream = self.connector.connect(&self.domain, stream).await?;
            Ok(NativeTlsStream(stream))
        })
    }
}

struct NativeTlsStream<S>(TokioNativeTlsStream<S>);

impl<S> AsyncRead for NativeTlsStream<S>
where
    TokioNativeTlsStream<S>: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl<S> AsyncWrite for NativeTlsStream<S>
where
    TokioNativeTlsStream<S>: AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl<S> TlsStream for NativeTlsStream<S>
where
    TokioNativeTlsStream<S>: AsyncRead + AsyncWrite + Unpin,
{
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}

#[cfg(test)]
mod tests {
    use super::{tls_verification_enabled, uses_tls};
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn tls_enabled_by_default() {
        let config: tokio_postgres::Config = "postgres://postgres:secret@db.internal:5432/app"
            .parse()
            .unwrap();
        assert!(uses_tls(&config));
    }

    #[test]
    fn tls_disabled_when_sslmode_disable() {
        let config: tokio_postgres::Config =
            "postgres://postgres:secret@db.internal:5432/app?sslmode=disable"
                .parse()
                .unwrap();
        assert!(!uses_tls(&config));
    }

    #[test]
    fn tls_verification_disabled_by_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("DB_SSL_VERIFY_CERT");
        assert!(!tls_verification_enabled());
    }

    #[test]
    fn tls_verification_enabled_with_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("DB_SSL_VERIFY_CERT", "true");
        assert!(tls_verification_enabled());
        std::env::remove_var("DB_SSL_VERIFY_CERT");
    }
}