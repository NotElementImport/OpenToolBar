mod emulator;

use futures_util::FutureExt;
use std::{sync::Arc, thread};
use tokio::join;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::{Connection, Result};

pub struct TouriSystemTray {}

impl TouriSystemTray {
    pub fn new() -> Arc<Self> {
        // Create instance and create Thread:
        let instance = Arc::new(Self {});

        let cloned_instance = instance.clone();
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                if let Err(err) = cloned_instance.start().await {
                    eprintln!("TouriSystemTray Err: {}", err);
                }
            });
        });

        instance
    }

    pub async fn start(&self) -> Result<()> {
        // Try find org.freedesktop.StatusNotifierWatcher or org.kded.StatusNotifierWatcher
        let connection = Connection::session().await?;
        let connection_proxy = DBusProxy::new(&connection).await?;

        let notifier_watcher_name = BusName::try_from("org.freedesktop.StatusNotifierWatcher")?;
        let kde_notifier_watcher_name = BusName::try_from("org.kde.StatusNotifierWatcher")?;

        let (notifier_exist, kde_notifier_exist) = join!(
            connection_proxy
                .name_has_owner(notifier_watcher_name)
                .map(|v| v.unwrap_or_default()),
            connection_proxy
                .name_has_owner(kde_notifier_watcher_name)
                .map(|v| v.unwrap_or_default()),
        );

        // If not, emulate it
        if !notifier_exist && !kde_notifier_exist {
            println!(
                "Notifier not found: using emulator. Debug value: {}",
                notifier_exist
            );

            emulator::SystemTrayEmulator::new();
        }

        Ok(())
    }
}
