use std::collections::VecDeque;

use crate::{AudioBackend, PlayerError, QueueItem};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayerState {
    #[default]
    Idle,
    Buffering,
    Playing,
    Paused,
    Ended,
}

pub struct Player<B> {
    backend: B,
    queue: VecDeque<QueueItem>,
    current: Option<QueueItem>,
    state: PlayerState,
}

impl<B: AudioBackend> Player<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            queue: VecDeque::new(),
            current: None,
            state: PlayerState::Idle,
        }
    }

    pub fn state(&self) -> PlayerState {
        self.state
    }

    pub fn current(&self) -> Option<&QueueItem> {
        self.current.as_ref()
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn enqueue(&mut self, item: QueueItem) {
        self.queue.push_back(item);
    }

    pub fn play(&mut self) -> Result<(), PlayerError> {
        match self.state {
            PlayerState::Idle | PlayerState::Ended => self.start_next(),
            PlayerState::Paused => {
                self.backend.play()?;
                self.state = PlayerState::Playing;
                Ok(())
            }
            PlayerState::Buffering | PlayerState::Playing => Ok(()),
        }
    }

    pub fn mark_ready(&mut self) -> Result<(), PlayerError> {
        if self.state != PlayerState::Buffering {
            return Err(PlayerError::InvalidState("not buffering"));
        }
        self.backend.play()?;
        self.state = PlayerState::Playing;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), PlayerError> {
        if self.state != PlayerState::Playing {
            return Err(PlayerError::InvalidState("not playing"));
        }
        self.backend.pause()?;
        self.state = PlayerState::Paused;
        Ok(())
    }

    pub fn skip_to_next(&mut self) -> Result<(), PlayerError> {
        if self.current.is_some() {
            self.backend.stop()?;
        }
        self.start_next()
    }

    fn start_next(&mut self) -> Result<(), PlayerError> {
        let Some(item) = self.queue.front().cloned() else {
            self.current = None;
            self.state = PlayerState::Ended;
            return Ok(());
        };
        self.backend.load(&item.source)?;
        self.queue.pop_front();
        self.current = Some(item);
        self.state = PlayerState::Buffering;
        Ok(())
    }
}
