use std::{future::Future, pin::Pin};

use zerobeat_core::Track;

use crate::CatalogError;

pub type QueueFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, CatalogError>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QueueRepeatMode {
    #[default]
    None,
    All,
    One,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct QueueStart {
    pub tracks: Vec<Track>,
    pub current_index: usize,
    pub track: Option<Track>,
    pub playlist_id: Option<String>,
    pub playlist_name: Option<String>,
    pub playlist_type: Option<String>,
    pub continuation: Option<String>,
    pub shuffle: bool,
    pub repeat_mode: QueueRepeatMode,
    pub endless_queue: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct QueueSession {
    pub id: String,
    pub state: String,
    pub tracks: Vec<Track>,
    pub current_index: usize,
    pub playlist_id: Option<String>,
    pub playlist_name: Option<String>,
    pub playlist_type: Option<String>,
    pub continuation: Option<String>,
    pub shuffle: bool,
    pub play_order: Vec<usize>,
    pub repeat_mode: QueueRepeatMode,
    pub endless_queue: bool,
    pub revision: i64,
}

pub trait MusicQueue: Send + Sync {
    fn active_queue(&self) -> QueueFuture<'_, Option<QueueSession>>;
    fn start_queue(&self, request: QueueStart) -> QueueFuture<'_, QueueSession>;
    fn get_queue(&self, session_id: &str) -> QueueFuture<'_, QueueSession>;
    fn delete_queue(&self, session_id: &str) -> QueueFuture<'_, ()>;
    fn next_queue(&self, session_id: &str) -> QueueFuture<'_, QueueSession>;
    fn previous_queue(&self, session_id: &str) -> QueueFuture<'_, QueueSession>;
    fn load_more_queue(&self, session_id: &str) -> QueueFuture<'_, QueueSession>;
    fn play_next_queue(&self, session_id: &str, track: Track) -> QueueFuture<'_, QueueSession>;
    fn add_queue(&self, session_id: &str, track: Track) -> QueueFuture<'_, QueueSession>;
    fn play_index_queue(&self, session_id: &str, index: usize) -> QueueFuture<'_, QueueSession>;
    fn remove_queue(&self, session_id: &str, index: usize) -> QueueFuture<'_, QueueSession>;
    fn clear_upcoming_queue(&self, session_id: &str) -> QueueFuture<'_, QueueSession>;
    fn set_shuffle_queue(&self, session_id: &str, enabled: bool) -> QueueFuture<'_, QueueSession>;
    fn set_repeat_queue(
        &self,
        session_id: &str,
        mode: QueueRepeatMode,
    ) -> QueueFuture<'_, QueueSession>;
}
