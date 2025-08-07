use futures_util::StreamExt;
use std::thread;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::RwLock;
use zbus::fdo::DBusProxy;
use zbus::{dbus_interface, ConnectionBuilder, SignalContext};
use zbus::{Connection, Result};

#[derive(Clone)]
struct Watcher {
    conn: Arc<Connection>,
    path: String,
    items: Arc<RwLock<HashSet<String>>>,
    hosts: Arc<RwLock<HashSet<String>>>,
}

impl Watcher {
    async fn new(conn: Arc<Connection>, path: impl Into<String>) -> Self {
        Self {
            conn,
            path: path.into(),
            items: Arc::new(RwLock::new(HashSet::new())),
            hosts: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    fn make_signal(&self) -> zbus::Result<SignalContext<'_>> {
        SignalContext::new(&*self.conn, self.path.as_str())
    }
}

#[dbus_interface(name = "org.freedesktop.StatusNotifierWatcher")]
impl Watcher {
    async fn RegisterStatusNotifierItem(&self, service: &str) -> zbus::fdo::Result<()> {
        let mut items = self.items.write().await;
        if items.insert(service.to_string()) {
            let ctx = self
                .make_signal()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            let _ = Self::StatusNotifierItemRegistered(&ctx, service).await;
        }
        Ok(())
    }

    async fn RegisterStatusNotifierHost(&self, service: &str) -> zbus::fdo::Result<()> {
        let mut hosts = self.hosts.write().await;
        if hosts.insert(service.to_string()) {
            let ctx = self
                .make_signal()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            let _ = Self::StatusNotifierHostRegistered(&ctx).await;
        }
        Ok(())
    }

    #[dbus_interface(property)]
    async fn RegisteredStatusNotifierItems(&self) -> Vec<String> {
        let items = self.items.read().await;
        items.iter().cloned().collect()
    }

    #[dbus_interface(property)]
    async fn IsStatusNotifierHostRegistered(&self) -> bool {
        let hosts = self.hosts.read().await;
        !hosts.is_empty()
    }

    #[dbus_interface(signal)]
    async fn StatusNotifierItemRegistered(
        ctx: &SignalContext<'_>,
        _service: &str,
    ) -> zbus::Result<()> {
        Ok(())
    }

    #[dbus_interface(signal)]
    async fn StatusNotifierItemUnregistered(
        ctx: &SignalContext<'_>,
        _service: &str,
    ) -> zbus::Result<()> {
        Ok(())
    }

    #[dbus_interface(signal)]
    async fn StatusNotifierHostRegistered(ctx: &SignalContext<'_>) -> zbus::Result<()> {
        Ok(())
    }
}

pub struct SystemTrayEmulator {}

impl SystemTrayEmulator {
    pub fn new() -> Arc<Self> {
        // Create new emulator for StatusNotifier
        let instance = Arc::new(Self {});

        let cloned_instance = instance.clone();
        thread::spawn(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                if let Err(err) = cloned_instance.start().await {
                    eprintln!("SystemTrayEmulator err: {}", err);
                }
            });
        });

        instance
    }

    async fn start(&self) -> Result<()> {
        // Create service:
        let connection = ConnectionBuilder::session()?
            .name("org.freedesktop.StatusNotifierWatcher")?
            .name("org.kde.StatusNotifierWatcher")?
            .build()
            .await?;

        let arc_conn = Arc::new(connection);
        // Create watcher:
        let watcher = Watcher::new(arc_conn.clone(), "/StatusNotifierWatcher").await;

        // Link command to watcher:
        let _ = arc_conn
            .object_server()
            .at("/StatusNotifierWatcher", watcher.clone())
            .await;

        // And create listener for removed items:
        let dbus_proxy = DBusProxy::new(&arc_conn.clone()).await?;
        let mut stream = dbus_proxy.receive_name_owner_changed().await?;
        let items = watcher.items.clone();

        while let Some(signal) = stream.next().await {
            if let Ok(args) = signal.args() {
                let name = args.name().clone();

                let old_owner_present = args.old_owner().as_ref().is_some();
                let new_owner_present = args.new_owner().as_ref().is_some();

                if old_owner_present && !new_owner_present {
                    let mut items = items.write().await;
                    let signal = watcher
                        .make_signal()
                        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;

                    if items.remove(&name.to_string()) {
                        let _ = Watcher::StatusNotifierItemUnregistered(&signal, &name).await;
                    }
                }
            }
        }

        Ok(())
    }
}
