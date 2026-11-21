//! Windows media keys and the system's now-playing card.

use std::ffi::c_void;
use std::sync::mpsc::{self, Receiver};

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig, SeekDirection};

pub enum MediaCommand {
    Toggle,
    Play,
    Pause,
    Next,
    Previous,
    SeekBy(i64),
}

pub struct Media {
    controls: MediaControls,
    events: Receiver<MediaCommand>,
}

impl Media {
    pub fn new(hwnd: *mut c_void, wake: impl Fn() + Send + 'static) -> Option<Self> {
        let config = PlatformConfig { display_name: "Nhịp", dbus_name: "nhip", hwnd: Some(hwnd) };
        let mut controls = MediaControls::new(config).ok()?;
        let (tx, events) = mpsc::channel();
        controls
            .attach(move |e: MediaControlEvent| {
                let cmd = match e {
                    MediaControlEvent::Toggle => Some(MediaCommand::Toggle),
                    MediaControlEvent::Play => Some(MediaCommand::Play),
                    MediaControlEvent::Pause => Some(MediaCommand::Pause),
                    MediaControlEvent::Next => Some(MediaCommand::Next),
                    MediaControlEvent::Previous => Some(MediaCommand::Previous),
                    MediaControlEvent::Seek(SeekDirection::Forward) => Some(MediaCommand::SeekBy(5000)),
                    MediaControlEvent::Seek(SeekDirection::Backward) => Some(MediaCommand::SeekBy(-5000)),
                    _ => None,
                };
                if let Some(cmd) = cmd {
                    let _ = tx.send(cmd);
                    wake();
                }
            })
            .ok()?;
        Some(Self { controls, events })
    }

    pub fn poll(&self) -> Option<MediaCommand> {
        self.events.try_recv().ok()
    }

    pub fn set_track(&mut self, title: &str, artist: &str, album: &str, duration_ms: u64) {
        let _ = self.controls.set_metadata(MediaMetadata {
            title: Some(title),
            artist: Some(artist),
            album: Some(album),
            duration: Some(std::time::Duration::from_millis(duration_ms)),
            cover_url: None,
        });
    }

    pub fn set_playing(&mut self, playing: bool) {
        let state = if playing { MediaPlayback::Playing { progress: None } } else { MediaPlayback::Paused { progress: None } };
        let _ = self.controls.set_playback(state);
    }
}
