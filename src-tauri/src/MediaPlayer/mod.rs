use debounced::Debounced;
use futures_channel::mpsc::{self, Sender};
use futures_util::StreamExt;
use serde::Serialize;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Wry};
use zbus::fdo::DBusProxy;
use zbus::{Connection, MessageStream};
use zvariant::Value;

pub struct TauriMediaPlayer {
    app_handle: AppHandle<Wry>,
}

#[derive(Clone, Serialize, Debug)]
struct MediaStruct {
    title: String,
    artist: Vec<String>,
    album: String,
    status: String,
}

impl TauriMediaPlayer {
    pub fn new(app_handle: AppHandle<Wry>) -> Arc<Self> {
        let instance = Arc::new(Self { app_handle });
        instance.clone().start();
        instance
    }

    fn start(self: Arc<Self>) {
        // Create debounce function, listen all changes with MediaPlayer2
        let sender = self.clone().create_emit_to_frontend();

        // Create thread and listen for changes from ZBus:
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                // Connect to ZBus and listen all changes
                let connection_to_bus = Connection::session().await.unwrap();

                let stream = MessageStream::from(connection_to_bus.clone());

                // Rule listen only: PropertiesChanged
                let dbus_proxy = DBusProxy::new(&connection_to_bus).await.unwrap();
                dbus_proxy
                    .add_match(
                        "type='signal',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'",
                    )
                    .await
                    .unwrap();

                // Listen events:
                if let Err(err) = self.listen_events(stream, sender).await {
                    eprintln!("TauriMediaPlayer err: {err}");
                }
            });
        });
    }

    fn create_emit_to_frontend(self: Arc<Self>) -> Sender<MediaStruct> {
        // Create channel for debounce:
        let (sender, receiver) = mpsc::channel::<MediaStruct>(1024);
        // Create debounce listener:
        let mut emit_event = Debounced::new(receiver, Duration::from_millis(100));

        // Create thread:
        let send_self = self.clone();
        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                // Listen all changes from sender channel:
                while let Some(media_info) = emit_event.next().await {
                    if let Ok(json_string) = serde_json::to_string(&media_info) {
                        // Send to frontend:
                        let _ = send_self.app_handle.emit("onUpdateMediaMeta", json_string);
                    }
                }
            });
        });

        sender
    }

    async fn listen_events(
        &self,
        mut stream: MessageStream,
        mut debounce_sender: Sender<MediaStruct>,
    ) -> zbus::Result<()> {
        // Media struct: (Using to send debounce)
        let mut media_info_struct = MediaStruct {
            title: "".to_string(),
            artist: Vec::new(),
            album: "".to_string(),
            status: "".to_string(),
        };

        // Await new message
        while let Some(event_message) = stream.next().await {
            if let Ok(event_message) = event_message {
                // If header member is PropertiesChanged
                let header = event_message.header()?;
                let member_as_str = match header.member().unwrap() {
                    Some(value) => value.as_str(),
                    None => "",
                };

                // If not PropertiesChanged skip
                if member_as_str != "PropertiesChanged" {
                    continue;
                }

                // Try parse body:
                if let Ok((body_interface, body_props, _)) = event_message.body::<(
                    String,
                    std::collections::HashMap<String, Value>,
                    Vec<String>,
                )>() {
                    // If is not MediaPlayer skip:
                    if !body_interface.starts_with("org.mpris.MediaPlayer2.Player") {
                        continue;
                    }

                    // Getting playing status:
                    let playing_status = match body_props.get("PlaybackStatus") {
                        Some(v) => v.to_string(),
                        None => String::new(),
                    };

                    let metadata = match body_props.get("Metadata") {
                        Some(Value::Dict(dict)) => Some(dict),
                        _ => None,
                    };

                    // Getting is Play state:
                    if !playing_status.is_empty() {
                        // Update, and send to debounce:
                        media_info_struct.status = playing_status.to_string().replace("\"", "");

                        if let Err(_) = debounce_sender.try_send(media_info_struct.clone()) {}
                    }
                    // Getting metadata:
                    else if let Some(metadata) = metadata {
                        let title = match metadata.get("xesam:title").unwrap() {
                            Some(Value::Str(v)) => v.to_string(),
                            _ => String::new(),
                        };

                        let album = match metadata.get("xesam:album").unwrap() {
                            Some(Value::Str(v)) => v.to_string(),
                            _ => String::new(),
                        };

                        let artist = match metadata.get("xesam:artist").unwrap() {
                            Some(Value::Array(arr)) => arr
                                .iter()
                                .filter_map(|v| v.downcast_ref::<str>().map(|a| a.to_string()))
                                .collect(),
                            _ => Vec::new(),
                        };

                        // Update, and send to debounce:
                        media_info_struct.artist = artist;
                        media_info_struct.album = album;
                        media_info_struct.title = title;

                        if let Err(err) = debounce_sender.try_send(media_info_struct.clone()) {
                            eprintln!("TauriMediaPlayer debounce err: {err}");
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
