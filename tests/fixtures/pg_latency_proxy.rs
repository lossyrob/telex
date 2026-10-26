use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

pub struct PgLatencyProxy {
    pub port: u16,
    delay_ms: Arc<AtomicU64>,
    connect_delay_ms: Arc<AtomicU64>,
    task: JoinHandle<std::io::Result<()>>,
}

impl PgLatencyProxy {
    pub async fn start(host: String, port: u16) -> Self {
        assert!(
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback()),
            "latency fixture requires a disposable loopback PostgreSQL server"
        );
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let local_port = listener.local_addr().unwrap().port();
        let delay_ms = Arc::new(AtomicU64::new(0));
        let delay = delay_ms.clone();
        let connect_delay_ms = Arc::new(AtomicU64::new(0));
        let connect_delay = connect_delay_ms.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (client, _) = accepted?;
                        let host = host.clone();
                        let delay = delay.clone();
                        let connect_delay = connect_delay.load(Ordering::SeqCst);
                        connections.spawn(async move {
                            if connect_delay != 0 {
                                tokio::time::sleep(Duration::from_millis(connect_delay)).await;
                            }
                            let server = TcpStream::connect((host.as_str(), port)).await?;
                            client.set_nodelay(true)?;
                            server.set_nodelay(true)?;
                            let (cr, cw) = client.into_split();
                            let (sr, sw) = server.into_split();
                            tokio::try_join!(pipe(cr, sw, delay.clone()), pipe(sr, cw, delay))?;
                            Ok::<_, std::io::Error>(())
                        });
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => {
                        result.expect("proxy connection task").expect("proxy connection I/O");
                    }
                }
            }
        });
        Self {
            port: local_port,
            delay_ms,
            connect_delay_ms,
            task,
        }
    }

    pub fn set_delay(&self, ms: u64) {
        self.delay_ms.store(ms, Ordering::SeqCst);
    }

    pub fn assert_running(&self) {
        assert!(!self.task.is_finished(), "PostgreSQL latency proxy stopped");
    }

    pub fn set_connect_delay(&self, ms: u64) {
        self.connect_delay_ms.store(ms, Ordering::SeqCst);
    }
}

impl Drop for PgLatencyProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn pipe(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    delay: Arc<AtomicU64>,
) -> std::io::Result<()> {
    let mut bytes = vec![0; 65536];
    loop {
        let size = reader.read(&mut bytes).await?;
        if size == 0 {
            return writer.shutdown().await;
        }
        let ms = delay.load(Ordering::SeqCst);
        if ms != 0 {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        writer.write_all(&bytes[..size]).await?;
    }
}
